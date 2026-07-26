package lume.ai.data

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.json.JsonObject
import lume.ai.data.crypto.CredencialGuardada
import lume.ai.data.crypto.CredentialStore
import lume.ai.data.remote.CanalAberto
import lume.ai.data.remote.ConviteDePareamento
import lume.ai.data.remote.Credencial
import lume.ai.data.remote.Endereco
import lume.ai.data.remote.EventoDoCanal
import lume.ai.data.remote.LumeClient
import lume.ai.data.remote.protocol.EnvelopeDeSaida
import lume.ai.data.remote.protocol.HistoryList
import lume.ai.data.remote.protocol.JsonDoProtocolo
import lume.ai.data.remote.protocol.MensagemDoServidor
import lume.ai.data.remote.protocol.PairRegister
import lume.ai.data.remote.protocol.ResolvePermission
import lume.ai.data.remote.protocol.SubmitPrompt
import lume.ai.data.remote.protocol.VERSAO_DO_PROTOCOLO
import lume.ai.data.remote.protocol.aplicarDelta
import lume.ai.domain.AgentSession
import lume.ai.domain.ConnectionState
import lume.ai.domain.HistoryCursor
import lume.ai.domain.HistoryPage
import lume.ai.domain.PermissionAction
import java.util.UUID
import javax.inject.Inject
import javax.inject.Singleton

/** Onde o pareamento está. A tela de Pareamento reage a isto. */
sealed interface EstadoDePareamento {
    data object Ocioso : EstadoDePareamento
    data object EmAndamento : EstadoDePareamento
    data object Concluido : EstadoDePareamento
    data class Falhou(val motivo: String) : EstadoDePareamento
}

/** Falha de uma requisição, já com o código do protocolo. */
class ErroDoProtocolo(val codigo: String, mensagem: String) : Exception(mensagem)

/**
 * O dono da conexão.
 *
 * Uma classe só sabe que existe um canal aberto, e é esta. Repositórios, ViewModels
 * e telas leem [connection] e [sessions] e nunca tocam num socket — é isso que faz
 * mover a conexão para dentro de um serviço em primeiro plano, na v2, não tocar em
 * nada acima daqui.
 *
 * ## Reconexão
 *
 * Backoff exponencial de 1s a 30s, e o relógio **zera quando o `ready` chega** —
 * não quando o socket abre. Um servidor que aceita o TCP e derruba no aperto de
 * mão não deve ser tratado como sucesso, senão o cliente entra em laço apertado
 * contra uma porta que não o quer.
 *
 * ## Requisição e resposta sobre um canal que só empurra
 *
 * O WebSocket não tem correlação nativa. O protocolo resolve isso com o `id` do
 * envelope, ecoado na resposta; aqui cada requisição registra um [CompletableDeferred]
 * num mapa e espera. O `result`/`error` que chegar com aquele `id` completa a espera.
 *
 * O mapa é limpo quando o canal cai: uma requisição pendente cuja conexão morreu
 * **precisa** falhar, e não ficar pendurada até o aplicativo ser fechado.
 */
@Singleton
class ConnectionManager @Inject constructor(
    private val cliente: LumeClient,
    private val credenciais: CredenciaisDeDesenvolvimento,
    private val credencialGuardada: CredentialStore,
) {

    private val escopo = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var laco: Job? = null

    @Volatile
    private var canal: CanalAberto? = null

    /** O convite da conexão em curso, quando ela é de pareamento. */
    @Volatile
    private var conviteEmCurso: ConviteDePareamento? = null

    private val pendentes = mutableMapOf<String, CompletableDeferred<Result<JsonObject?>>>()
    private val cadeado = Mutex()

    private val _connection = MutableStateFlow<ConnectionState>(ConnectionState.Desconectado())
    val connection: StateFlow<ConnectionState> = _connection.asStateFlow()

    private val _sessions = MutableStateFlow<List<AgentSession>>(emptyList())
    val sessions: StateFlow<List<AgentSession>> = _sessions.asStateFlow()

    /** Nome do desktop, vindo do `ready`. Antes disso não há o que mostrar. */
    private val _hostname = MutableStateFlow<String?>(null)
    val hostname: StateFlow<String?> = _hostname.asStateFlow()

    private val _pareamento = MutableStateFlow<EstadoDePareamento>(EstadoDePareamento.Ocioso)
    val pareamento: StateFlow<EstadoDePareamento> = _pareamento.asStateFlow()

    fun conectar() {
        if (laco?.isActive == true) return
        laco = escopo.launch {
            val alvo = alvoGuardado() ?: credenciais.alvo()
            if (alvo == null) {
                // Sem credencial o aplicativo não está quebrado — ele está
                // desemparelhado, que é estado legítimo e tem tela própria.
                _connection.value = ConnectionState.Desconectado()
                return@launch
            }
            laçoDeConexao(alvo)
        }
    }

    /**
     * A credencial guardada vira alvo.
     *
     * Vem antes da de desenvolvimento de propósito: um aparelho pareado de
     * verdade não deve voltar a usar o token do `local.properties` só porque ele
     * continua no build.
     */
    private suspend fun alvoGuardado(): Alvo? {
        val guardada = credencialGuardada.ler() ?: return null
        val fingerprint = deHex(guardada.fingerprintEmHex) ?: return null
        val host = guardada.candidatos.firstOrNull() ?: return null
        return Alvo(
            endereco = Endereco(host, guardada.porta),
            credencial = Credencial.Token(guardada.token),
            fingerprint = fingerprint,
        )
    }

    /**
     * Pareia com o desktop cujo QR foi lido.
     *
     * O código é consumido **no aperto de mão**, não no `pair.register` — verificar
     * e consumir são uma operação única sob o mesmo cadeado do lado do servidor.
     * A consequência prática: uma foto do QR vale para **uma** negociação
     * bem-sucedida, e cair entre o `101` e o registro obriga a ler o código novo,
     * que a janela do desktop já regenerou.
     *
     * Os candidatos são tentados **na ordem recebida**: o desktop já os ordenou
     * com interfaces físicas antes das virtuais, e uma máquina com Docker anuncia
     * `172.17.0.1`, que não leva a lugar nenhum vindo de fora.
     */
    fun parear(convite: ConviteDePareamento, nomeDoAparelho: String) {
        if (laco?.isActive == true) return
        _pareamento.value = EstadoDePareamento.EmAndamento
        laco = escopo.launch {
            for (endereco in convite.enderecos()) {
                val alvo = Alvo(
                    endereco = endereco,
                    credencial = Credencial.CodigoDePareamento(convite.codigo),
                    fingerprint = convite.fingerprint,
                )
                val pareou = umaTentativa(alvo, registrarComo = nomeDoAparelho, convite = convite)
                if (pareou) {
                    // A partir daqui a credencial guardada manda, e a reconexão
                    // usa o token. O código já foi consumido e não serve mais.
                    val comToken = alvoGuardado() ?: return@launch
                    laçoDeConexao(comToken)
                    return@launch
                }
            }
            _pareamento.value = EstadoDePareamento.Falhou(
                if (convite.candidatos.isEmpty()) {
                    "O computador não informou um endereço. Digite o endereço."
                } else {
                    "Não foi possível alcançar o computador"
                },
            )
            _connection.value = ConnectionState.Desconectado()
        }
    }

    fun desconectar() {
        laco?.cancel()
        laco = null
        canal?.fechar()
        canal = null
        escopo.launch { falharPendentes(ErroDoProtocolo("desconectado", "A conexão foi encerrada")) }
        _connection.value = ConnectionState.Desconectado(ultimoContato = null)
    }

    private suspend fun laçoDeConexao(alvo: Alvo) {
        var espera = ESPERA_INICIAL
        while (true) {
            _connection.value = ConnectionState.Conectando
            val chegouAoReady = umaTentativa(alvo)
            // Só sucesso de protocolo zera o backoff. Ver o comentário da classe.
            espera = if (chegouAoReady) ESPERA_INICIAL else (espera * 2).coerceAtMost(ESPERA_MAXIMA)
            delay(espera)
        }
    }

    /**
     * Uma conexão, do início ao fim dela.
     *
     * @param registrarComo quando presente, esta é a conexão de pareamento: o
     *   `pair.register` sai assim que o canal abre, e o `pair.accepted` grava a
     *   credencial. O servidor emenda o `ready` **na mesma conexão**, sem exigir
     *   reconexão — por isso o mesmo `collect` segue valendo depois.
     * @return `true` se a conexão cumpriu seu propósito: `ready` numa conexão
     *   comum, credencial gravada numa de pareamento.
     */
    private suspend fun umaTentativa(
        alvo: Alvo,
        registrarComo: String? = null,
        convite: ConviteDePareamento? = null,
    ): Boolean {
        var pronto = false
        conviteEmCurso = convite
        cliente.conectar(alvo.endereco, alvo.credencial, alvo.fingerprint).collect { evento ->
            when (evento) {
                is EventoDoCanal.Aberto -> {
                    canal = evento.canal
                    // Prazo de 10 segundos do lado do servidor: quem consumiu um
                    // código e não se registra está segurando conexão sem ser
                    // ninguém. Mandar já no `onOpen` é o que cabe nele.
                    if (registrarComo != null) enviarRegistro(evento.canal, registrarComo)
                }

                is EventoDoCanal.Recebida -> {
                    if (tratar(evento.mensagem)) pronto = true
                }

                is EventoDoCanal.Encerrado -> {
                    canal = null
                    falharPendentes(
                        ErroDoProtocolo("desconectado", evento.motivo ?: "A conexão caiu"),
                    )
                    _connection.value = estadoDeQueda(evento)
                }
            }
        }
        canal = null
        conviteEmCurso = null
        // Numa conexão de pareamento o que importa é a credencial ter sido
        // gravada; numa comum, ter chegado ao `ready`.
        return if (convite != null) _pareamento.value == EstadoDePareamento.Concluido else pronto
    }

    /**
     * Como uma queda é contada ao usuário.
     *
     * A distinção que importa: `401` e `426` são recusa, e recusa não melhora
     * tentando de novo — o usuário precisa agir. Queda de rede é situação, e o
     * cache continua valendo enquanto o backoff trabalha.
     */
    private fun estadoDeQueda(evento: EventoDoCanal.Encerrado): ConnectionState = when (evento.httpStatus) {
        401 -> ConnectionState.Erro("Este aparelho não está mais pareado")
        426 -> ConnectionState.Erro("Atualize o Lume no computador")
        else -> ConnectionState.Desconectado(ultimoContato = null)
    }

    /** @return `true` quando a mensagem foi o `ready`. */
    private suspend fun tratar(mensagem: MensagemDoServidor): Boolean {
        when (mensagem) {
            is MensagemDoServidor.Pronto -> {
                // Versão diferente é incompatibilidade de verdade, e o documento
                // manda dizer isso em vez de falhar de forma opaca.
                if (mensagem.payload.protocolVersion != VERSAO_DO_PROTOCOLO) {
                    _connection.value = ConnectionState.Erro("Atualize o Lume no computador")
                    canal?.fechar()
                    return false
                }
                _hostname.value = mensagem.payload.hostname
                _connection.value = ConnectionState.Conectado
                return true
            }

            // Substitui, não mescla: sessão que sumiu enquanto o aplicativo estava
            // fechado não pode sobreviver a uma reconexão.
            is MensagemDoServidor.Snapshot -> _sessions.value = mensagem.payload.sessions

            is MensagemDoServidor.Delta -> _sessions.value =
                aplicarDelta(_sessions.value, mensagem.payload)

            is MensagemDoServidor.Resultado -> completar(mensagem.id, Result.success(mensagem.payload))

            is MensagemDoServidor.Falha -> {
                val erro = ErroDoProtocolo(mensagem.payload.code, mensagem.payload.message)
                if (mensagem.id == null) {
                    // Erro sem `id` é de conexão, não de requisição. `revoked` é o
                    // caso concreto: o desktop revogou este aparelho agora.
                    _connection.value = ConnectionState.Erro(mensagem.payload.message)
                } else {
                    completar(mensagem.id, Result.failure(erro))
                }
            }

            is MensagemDoServidor.Pareado -> {
                // Gravado **antes** de qualquer outra coisa: o token trafega uma
                // única vez, e o desktop guarda só o SHA-256 dele. Perder aqui
                // significa parear de novo, não recuperar.
                conviteEmCurso?.let { convite ->
                    credencialGuardada.gravar(
                        CredencialGuardada(
                            deviceId = mensagem.payload.deviceId,
                            token = mensagem.payload.token,
                            fingerprintEmHex = convite.fingerprint.joinToString("") { "%02x".format(it) },
                            candidatos = convite.candidatos,
                            porta = convite.porta,
                            nomeDoDesktop = convite.nomeDaMaquina,
                            pareadoEm = rotuloDeHoje(),
                        ),
                    )
                    _pareamento.value = EstadoDePareamento.Concluido
                }
            }
            is MensagemDoServidor.Desconhecida -> Unit
        }
        return false
    }

    suspend fun resolverPermissao(sessionId: String, permissionId: String, action: PermissionAction) {
        requisitar(
            tipo = "permission.resolve",
            payload = ResolvePermission(sessionId, permissionId, action),
            serializador = ResolvePermission.serializer(),
        )
    }

    suspend fun enviarPrompt(sessionId: String, prompt: String) {
        requisitar(
            tipo = "prompt.submit",
            payload = SubmitPrompt(sessionId, prompt),
            serializador = SubmitPrompt.serializer(),
        )
    }

    /**
     * Uma página do histórico.
     *
     * O payload de `result` depende da requisição — o envelope não diz qual forma
     * esperar, quem sabe é o `id` que guardamos. Por isso a decodificação acontece
     * aqui, onde o tipo do pedido ainda é conhecido, e não no leitor genérico.
     */
    suspend fun listarHistorico(limit: Int, before: HistoryCursor?): HistoryPage {
        val payload = requisitar(
            tipo = "history.list",
            payload = HistoryList(limit, before),
            serializador = HistoryList.serializer(),
        ) ?: throw ErroDoProtocolo("internal", "Resposta de histórico sem conteúdo")
        return JsonDoProtocolo.decodeFromJsonElement(HistoryPage.serializer(), payload)
    }

    /**
     * Envia e espera a resposta com o mesmo `id`.
     *
     * O prazo existe porque o servidor pode simplesmente nunca responder — e uma
     * corrotina de interface esperando para sempre é um botão que nunca volta ao
     * normal. O `id` é UUID v4 gerado aqui, e é o mesmo que sustenta a
     * idempotência do lado do servidor: reenviar com o `id` de antes devolve o
     * resultado guardado em vez de executar de novo.
     */
    private suspend fun <T> requisitar(
        tipo: String,
        payload: T,
        serializador: kotlinx.serialization.KSerializer<T>,
    ): JsonObject? {
        val aberto = canal ?: throw ErroDoProtocolo("desconectado", "Sem conexão com o computador")
        val id = UUID.randomUUID().toString()
        val espera = CompletableDeferred<Result<JsonObject?>>()
        cadeado.withLock { pendentes[id] = espera }

        val texto = JsonDoProtocolo.encodeToString(
            EnvelopeDeSaida.serializer(serializador),
            EnvelopeDeSaida(tipo = tipo, id = id, payload = payload),
        )
        if (!aberto.enviar(texto)) {
            cadeado.withLock { pendentes.remove(id) }
            throw ErroDoProtocolo("desconectado", "Não foi possível enviar")
        }

        val resposta = withTimeoutOrNull(PRAZO_DE_RESPOSTA) { espera.await() }
        cadeado.withLock { pendentes.remove(id) }
        return (resposta ?: throw ErroDoProtocolo("timeout", "O computador não respondeu"))
            .getOrThrow()
    }

    private suspend fun completar(id: String?, resultado: Result<JsonObject?>) {
        if (id == null) return
        cadeado.withLock { pendentes.remove(id) }?.complete(resultado)
    }

    private suspend fun falharPendentes(erro: Throwable) {
        cadeado.withLock {
            pendentes.values.forEach { it.complete(Result.failure(erro)) }
            pendentes.clear()
        }
    }

    private fun enviarRegistro(aberto: CanalAberto, nome: String) {
        val texto = JsonDoProtocolo.encodeToString(
            EnvelopeDeSaida.serializer(PairRegister.serializer()),
            EnvelopeDeSaida(
                tipo = "pair.register",
                id = UUID.randomUUID().toString(),
                payload = PairRegister(deviceName = nome, platform = "android"),
            ),
        )
        aberto.enviar(texto)
    }

    /** "12 jul", como a tela de Ajustes mostra. */
    private fun rotuloDeHoje(): String = java.time.LocalDate.now()
        .format(java.time.format.DateTimeFormatter.ofPattern("d MMM", java.util.Locale("pt", "BR")))

    private fun deHex(texto: String): ByteArray? = runCatching {
        ByteArray(texto.length / 2) { texto.substring(it * 2, it * 2 + 2).toInt(16).toByte() }
    }.getOrNull()?.takeIf { it.size == 32 }

    private companion object {
        const val ESPERA_INICIAL = 1_000L
        const val ESPERA_MAXIMA = 30_000L
        const val PRAZO_DE_RESPOSTA = 15_000L
    }
}
