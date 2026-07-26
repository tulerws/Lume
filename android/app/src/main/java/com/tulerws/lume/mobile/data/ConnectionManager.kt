package com.tulerws.lume.mobile.data

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.json.JsonObject
import com.tulerws.lume.mobile.data.crypto.CredencialGuardada
import com.tulerws.lume.mobile.data.crypto.CredentialStore
import com.tulerws.lume.mobile.data.remote.CanalAberto
import com.tulerws.lume.mobile.data.remote.ConviteDePareamento
import com.tulerws.lume.mobile.data.remote.Credencial
import com.tulerws.lume.mobile.data.remote.Endereco
import com.tulerws.lume.mobile.data.remote.EventoDoCanal
import com.tulerws.lume.mobile.data.remote.LumeClient
import com.tulerws.lume.mobile.data.remote.protocol.EnvelopeDeSaida
import com.tulerws.lume.mobile.data.remote.protocol.HistoryList
import com.tulerws.lume.mobile.data.remote.protocol.JsonDoProtocolo
import com.tulerws.lume.mobile.data.remote.protocol.MensagemDoServidor
import com.tulerws.lume.mobile.data.remote.protocol.PairRegister
import com.tulerws.lume.mobile.data.remote.protocol.ResolvePermission
import com.tulerws.lume.mobile.data.remote.protocol.SubmitPrompt
import com.tulerws.lume.mobile.data.remote.protocol.VERSAO_DO_PROTOCOLO
import com.tulerws.lume.mobile.data.remote.protocol.aplicarDelta
import com.tulerws.lume.mobile.domain.AgentSession
import com.tulerws.lume.mobile.domain.ConnectionState
import com.tulerws.lume.mobile.domain.HistoryCursor
import com.tulerws.lume.mobile.domain.HistoryPage
import com.tulerws.lume.mobile.domain.PermissionAction
import java.util.UUID
import javax.inject.Inject
import javax.inject.Singleton

/**
 * O que uma tentativa de pareamento produziu.
 *
 * **Resultado, não estado.** A versão anterior guardava o estado num `StateFlow`
 * do gerente — que é `@Singleton` e vive enquanto o processo vive — para
 * descrever uma tentativa feita por **uma tela**. Nada devolvia o valor a
 * "ocioso", então depois do primeiro pareamento a tela ficava inutilizável: ao
 * reabrir, ela lia "concluído" no primeiro quadro e saía sozinha; depois de uma
 * falha, o leitor recusava toda leitura seguinte.
 *
 * Devolvendo resultado, não há o que resetar: o valor nasce e morre com a
 * chamada, e quem guarda estado é a tela, que morre junto.
 */
/**
 * O que a tela de pareamento precisa do mundo: **um método**.
 *
 * Existe porque `PairViewModel` dependia do `ConnectionManager` inteiro, e
 * testá-lo exigia construir um gerente com `CredentialStore`, que exige
 * `Context`. Uma tela que só sabe parear não deveria arrastar a conexão inteira
 * para dentro de um teste de unidade.
 */
interface Pareador {
    suspend fun parear(
        convite: ConviteDePareamento,
        nomeDoAparelho: String,
    ): ResultadoDoPareamento
}

sealed interface ResultadoDoPareamento {
    data object Pareado : ResultadoDoPareamento
    data class Falhou(val motivo: String) : ResultadoDoPareamento
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
) : Pareador {

    private val escopo = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var laco: Job? = null

    @Volatile
    private var canal: CanalAberto? = null

    /** O convite da conexão em curso, quando ela é de pareamento. */
    @Volatile
    private var conviteEmCurso: ConviteDePareamento? = null

    /**
     * Motivo de uma recusa que **não melhora tentando de novo**.
     *
     * Preenchido pelos caminhos terminais — `revoked`, `401`, versão
     * incompatível — e lido pelo laço, que encerra em vez de reagendar.
     */
    @Volatile
    private var recusaTerminal: String? = null

    /** Se o `pair.accepted` da tentativa em curso conseguiu gravar. */
    @Volatile
    private var credencialFoiGravada: Boolean = false

    /** Se, ao sair por recusa, a credencial deve ser apagada. Só `revoked`. */
    @Volatile
    private var apagarCredencialAoSair: Boolean = false

    private val pendentes = mutableMapOf<String, CompletableDeferred<Result<JsonObject?>>>()
    private val cadeado = Mutex()

    private val _connection = MutableStateFlow<ConnectionState>(ConnectionState.Desconectado())
    val connection: StateFlow<ConnectionState> = _connection.asStateFlow()

    private val _sessions = MutableStateFlow<List<AgentSession>>(emptyList())
    val sessions: StateFlow<List<AgentSession>> = _sessions.asStateFlow()

    fun conectar() {
        if (laco?.isActive == true) return
        laco = escopo.launch {
            // `runCatching` porque `alvosGuardados` toca DataStore e Keystore, e
            // os dois lançam — arquivo corrompido, chave invalidada quando o
            // usuário acrescenta bloqueio de tela. Sem isto a exceção escapa do
            // `launch` e derruba o processo no arranque.
            val alvos = runCatching { alvosGuardados() }.getOrDefault(emptyList())
                .ifEmpty { listOfNotNull(credenciais.alvo()) }
            if (alvos.isEmpty()) {
                // Sem credencial o aplicativo não está quebrado — ele está
                // desemparelhado, que é estado legítimo e tem tela própria.
                _connection.value = ConnectionState.Desconectado()
                return@launch
            }
            laçoDeConexao(alvos)
        }
    }

    /**
     * A credencial guardada vira alvo.
     *
     * Vem antes da de desenvolvimento de propósito: um aparelho pareado de
     * verdade não deve voltar a usar o token do `local.properties` só porque ele
     * continua no build.
     */
    private suspend fun alvosGuardados(): List<Alvo> {
        val guardada = credencialGuardada.ler() ?: return emptyList()
        val fingerprint = deHex(guardada.fingerprintEmHex) ?: return emptyList()
        // Último que funcionou primeiro, depois os demais na ordem do QR — que o
        // desktop já ordenou com interfaces físicas antes das virtuais. Fixar o
        // índice 0, como a versão anterior fazia, escolhia justamente o endereço
        // que podia ter acabado de falhar no pareamento.
        val ordenados = (listOfNotNull(guardada.ultimoQueFuncionou) + guardada.candidatos).distinct()
        return ordenados.map { host ->
            Alvo(
                endereco = Endereco(host, guardada.porta),
                credencial = Credencial.Token(guardada.token),
                fingerprint = fingerprint,
            )
        }
    }

    /** Registra o endereço que respondeu, para a próxima reconexão começar por ele. */
    private suspend fun lembrarEndereco(host: String) {
        val guardada = credencialGuardada.ler() ?: return
        if (guardada.ultimoQueFuncionou == host) return
        runCatching { credencialGuardada.gravar(guardada.copy(ultimoQueFuncionou = host)) }
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
    override suspend fun parear(
        convite: ConviteDePareamento,
        nomeDoAparelho: String,
    ): ResultadoDoPareamento {
        laco?.cancelAndJoin()
        for (endereco in convite.enderecos()) {
            val alvo = Alvo(
                endereco = endereco,
                credencial = Credencial.CodigoDePareamento(convite.codigo),
                fingerprint = convite.fingerprint,
            )
            val pareou = runCatching {
                umaTentativa(alvo, registrarComo = nomeDoAparelho, convite = convite)
            }.getOrDefault(false)
            if (pareou) {
                runCatching { lembrarEndereco(endereco.host) }
                // A partir daqui a credencial guardada manda, e a reconexão usa o
                // token. O código já foi consumido e não serve mais.
                laco = escopo.launch { laçoDeConexao(alvosGuardados()) }
                return ResultadoDoPareamento.Pareado
            }
        }
        _connection.value = ConnectionState.Desconectado()
        return ResultadoDoPareamento.Falhou(
            if (convite.candidatos.isEmpty()) {
                "O computador não informou um endereço. Digite o endereço."
            } else {
                "Não foi possível alcançar o computador"
            },
        )
    }

    fun desconectar() {
        val anterior = laco
        laco = null
        canal?.fechar()
        escopo.launch {
            // **Espera o laço antigo morrer** antes de considerar a conexão
            // encerrada. Sem o `join`, a corrotina cancelada ainda estava
            // desenrolando e executava `canal = null` — apagando a referência de
            // um canal novo já aberto. A partir dali toda ação falhava com
            // "Sem conexão" enquanto a interface mostrava "Conectado".
            anterior?.cancelAndJoin()
            falharPendentes(ErroDoProtocolo("desconectado", "A conexão foi encerrada"))
        }
        _connection.value = ConnectionState.Desconectado(ultimoContato = null)
    }

    /**
     * Mantém uma conexão viva, percorrendo os endereços.
     *
     * **Tem saída.** Recusa do servidor — `revoked`, `401`, versão incompatível —
     * encerra o laço e deixa o `Erro` na tela. Antes ele era `while (true)`, e o
     * `Erro` durava um quadro: a iteração seguinte reescrevia `Conectando` dois
     * segundos depois, em ciclo eterno contra uma porta trancada de propósito.
     */
    private suspend fun laçoDeConexao(alvos: List<Alvo>) {
        if (alvos.isEmpty()) {
            _connection.value = ConnectionState.Desconectado()
            return
        }
        var espera = ESPERA_INICIAL
        var indice = 0
        while (true) {
            val alvo = alvos[indice % alvos.size]
            _connection.value = ConnectionState.Conectando
            recusaTerminal = null
            val chegouAoReady = runCatching { umaTentativa(alvo) }.getOrDefault(false)

            recusaTerminal?.let { motivo ->
                // Recusa não melhora tentando de novo — o usuário precisa agir.
                _connection.value = ConnectionState.Erro(motivo)
                if (apagarCredencialAoSair) {
                    runCatching { credencialGuardada.apagar() }
                }
                return
            }

            if (chegouAoReady) {
                runCatching { lembrarEndereco(alvo.endereco.host) }
                espera = ESPERA_INICIAL
                // Deu certo aqui: continua neste endereço na próxima volta.
            } else {
                espera = (espera * 2).coerceAtMost(ESPERA_MAXIMA)
                indice += 1
            }
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
        var meuCanal: CanalAberto? = null
        credencialFoiGravada = false
        conviteEmCurso = convite
        cliente.conectar(alvo.endereco, alvo.credencial, alvo.fingerprint).collect { evento ->
            when (evento) {
                is EventoDoCanal.Aberto -> {
                    meuCanal = evento.canal
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
                    // Só limpa se ainda for o **meu** canal: uma tentativa
                    // encerrando não pode apagar a referência de outra que já
                    // abriu.
                    if (canal === meuCanal) canal = null
                    falharPendentes(
                        ErroDoProtocolo("desconectado", evento.motivo ?: "A conexão caiu"),
                    )
                    _connection.value = estadoDeQueda(evento)
                }
            }
        }
        if (canal === meuCanal) canal = null
        conviteEmCurso = null
        // Numa conexão de pareamento o que importa é a credencial ter sido
        // gravada; numa comum, ter chegado ao `ready`.
        return if (convite != null) credencialFoiGravada else pronto
    }

    /**
     * Como uma queda é contada ao usuário.
     *
     * A distinção que importa: `401` e `426` são recusa, e recusa não melhora
     * tentando de novo — o usuário precisa agir. Queda de rede é situação, e o
     * cache continua valendo enquanto o backoff trabalha.
     */
    private fun estadoDeQueda(evento: EventoDoCanal.Encerrado): ConnectionState {
        when (evento.httpStatus) {
            // `401` para o laço mas **não** apaga: normalmente é revogação, mas
            // pode ser defeito momentâneo, e apagar é irreversível — parear de
            // novo exige acesso físico ao desktop.
            401 -> recusaTerminal = "Este aparelho não está mais pareado"
            426 -> recusaTerminal = "Atualize o Lume no computador"
        }
        // Recusa terminal já registrada não é sobrescrita: quem decide o estado
        // final é o laço, ao sair.
        return if (recusaTerminal != null) {
            _connection.value
        } else {
            ConnectionState.Desconectado(ultimoContato = null)
        }
    }

    /** @return `true` quando a mensagem foi o `ready`. */
    private suspend fun tratar(mensagem: MensagemDoServidor): Boolean {
        when (mensagem) {
            is MensagemDoServidor.Pronto -> {
                // Versão diferente é incompatibilidade de verdade, e o documento
                // manda dizer isso em vez de falhar de forma opaca.
                if (mensagem.payload.protocolVersion != VERSAO_DO_PROTOCOLO) {
                    // Marca **antes** de fechar. Definir o `Erro` aqui e fechar em
                    // seguida não funcionava: o `Encerrado` que o próprio
                    // fechamento gera sobrescrevia com `Desconectado`, e a
                    // mensagem não durava um quadro.
                    recusaTerminal = "Atualize o Lume no computador"
                    canal?.fechar()
                    return false
                }
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
                    // caso concreto: o desktop revogou este aparelho agora, e o
                    // token está provadamente morto — bater de novo é inútil.
                    recusaTerminal = mensagem.payload.message
                    apagarCredencialAoSair = mensagem.payload.code == "revoked"
                } else {
                    completar(mensagem.id, Result.failure(erro))
                }
            }

            is MensagemDoServidor.Pareado -> credencialFoiGravada = runCatching {
                // Gravado **antes** de qualquer outra coisa: o token trafega uma
                // única vez, e o desktop guarda só o SHA-256 dele. Perder aqui
                // significa parear de novo, não recuperar.
                val convite = conviteEmCurso
                if (convite != null) {
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
                    true
                } else {
                    false
                }
                // Keystore e DataStore lançam: chave invalidada por o usuário ter
                // acrescentado bloqueio de tela, arquivo corrompido. Sem captura,
                // isso fechava o aplicativo **no `pair.accepted`** — o ponto que
                // o comentário abaixo descreve como impossível de repetir.
            }.getOrDefault(false).also { gravou ->
                // Encerra a conexão do pareamento assim que a credencial existe.
                //
                // Sem isto o pareamento **terminava e a tela não saía do estado de
                // carregando**. `umaTentativa` espera o `collect` do canal, e esse
                // `collect` só volta quando a conexão morre: a credencial era
                // gravada, o desktop registrava o aparelho, e `parear()` seguia
                // suspensa para sempre atendendo a conexão. Só reabrir o
                // aplicativo destravava, porque aí a credencial já estava no disco.
                //
                // Fechar aqui é o que o comentário de `parear()` sempre descreveu:
                // esta conexão foi autenticada pelo **código**, o código acabou de
                // ser consumido, e quem assume é o laço com o **token**. O custo é
                // um aperto de mão a mais, na rede local, logo depois de a pessoa
                // ter apontado a câmera.
                if (gravou) canal?.fechar()
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
