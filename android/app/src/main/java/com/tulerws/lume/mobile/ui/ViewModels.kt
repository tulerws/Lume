package com.tulerws.lume.mobile.ui

import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import com.tulerws.lume.mobile.data.ConnectionManager
import com.tulerws.lume.mobile.data.ErroDoProtocolo
import com.tulerws.lume.mobile.data.Pareador
import com.tulerws.lume.mobile.data.ResultadoDoPareamento
import com.tulerws.lume.mobile.data.PreferenciasEmMemoria
import com.tulerws.lume.mobile.data.remote.FalhaDeLeitura
import com.tulerws.lume.mobile.data.remote.LeituraDeConviteFalhou
import com.tulerws.lume.mobile.data.remote.lerConvite
import com.tulerws.lume.mobile.data.update.Atualizador
import com.tulerws.lume.mobile.data.update.EstadoDaAtualizacao
import com.tulerws.lume.mobile.data.repo.HistoryRepository
import com.tulerws.lume.mobile.data.repo.PairingRepository
import com.tulerws.lume.mobile.data.repo.SessionRepository
import com.tulerws.lume.mobile.domain.AgentSession
import com.tulerws.lume.mobile.domain.ConnectionState
import com.tulerws.lume.mobile.domain.HistoryCursor
import com.tulerws.lume.mobile.domain.HistoryEntry
import com.tulerws.lume.mobile.domain.PairedDesktop
import com.tulerws.lume.mobile.domain.PermissionAction
import com.tulerws.lume.mobile.ui.session.PermissaoResolvida
import com.tulerws.lume.mobile.ui.theme.ThemeMode
import javax.inject.Inject

/**
 * Traduz uma exceção em código do protocolo, **sem engolir cancelamento**.
 *
 * Capturar `Exception` largo é necessário aqui: além do `ErroDoProtocolo`, a
 * decodificação de uma página lança `SerializationException`, e o DataStore lança
 * `IOException` — nenhum dos dois é do protocolo, e qualquer um que escape de um
 * `launch` derruba o processo.
 *
 * Mas `CancellationException` **precisa** subir: ela é como uma corrotina é
 * cancelada, e tratá-la como falha faria o ViewModel registrar erro toda vez que
 * a tela é fechada, além de quebrar o cancelamento estruturado.
 */
private fun codigoDe(erro: Throwable): String {
    if (erro is kotlinx.coroutines.CancellationException) throw erro
    return (erro as? ErroDoProtocolo)?.codigo ?: "internal"
}

/**
 * Em qual dos três momentos a tela de sessão está.
 *
 * [Encerrada] carrega a última sessão conhecida de propósito: a tela continua
 * mostrando o que o usuário estava lendo, com uma faixa dizendo o que houve.
 * Fazê-la desaparecer por baixo de quem lia seria a mesma mecânica que causou o
 * defeito de tocar numa sessão e não ir a lugar nenhum.
 */
sealed interface EstadoDaSessao {
    data object Carregando : EstadoDaSessao
    data class Ativa(val sessao: AgentSession) : EstadoDaSessao
    data class Encerrada(val ultimaConhecida: AgentSession) : EstadoDaSessao
}

/**
 * Estado que sobrevive a rotação e à recriação da activity.
 *
 * Um ViewModel por tela, e não um só para o aplicativo inteiro: a tela de Sessão
 * precisa de `SavedStateHandle` para saber qual sessão abriu, e a de Histórico
 * carrega paginação que não faz sentido viver enquanto ninguém a olha. O único
 * compartilhado é [AppViewModel], porque o tema envolve tudo.
 */

/**
 * Tema e nome do desktop — as duas coisas que existem antes de qualquer tela.
 *
 * O tema mora aqui e não numa tela porque `LumeTheme` envolve o `NavHost` inteiro.
 * Lê-lo dentro de Ajustes obrigaria a empurrar o valor para cima, que é o
 * caminho contrário ao do dado.
 */
@HiltViewModel
class AppViewModel @Inject constructor(
    private val preferencias: PreferenciasEmMemoria,
    pareamento: PairingRepository,
    atualizador: Atualizador,
) : ViewModel() {

    val tema: StateFlow<ThemeMode> = preferencias.tema

    val desktop: StateFlow<PairedDesktop?> = pareamento.desktop

    init {
        // A checagem de abertura mora aqui porque este ViewModel é criado uma vez
        // por processo, com o aplicativo — não a cada visita a Ajustes. Pô-la no
        // `SettingsViewModel` faria a verificação acontecer só para quem abre
        // Ajustes, que é justamente quem já iria procurar por atualização sozinho.
        //
        // Não há `try` em volta: `verificar()` já converte qualquer falha em
        // `EstadoDaAtualizacao.Falhou`, e uma exceção escapando de um `launch`
        // derruba o processo. A represa de seis horas vive no [Atualizador].
        viewModelScope.launch { atualizador.verificarSePassouTempo() }
    }

    fun definirTema(modo: ThemeMode) = preferencias.definirTema(modo)
}

@HiltViewModel
class SessionsViewModel @Inject constructor(
    private val sessoesRepo: SessionRepository,
) : ViewModel() {

    val conexao: StateFlow<ConnectionState> = sessoesRepo.connection
    val sessoes: StateFlow<List<AgentSession>> = sessoesRepo.sessions

    fun tentarNovamente() = sessoesRepo.reconectar()
}

/**
 * Uma sessão, escolhida pelo identificador que veio na rota.
 *
 * A sessão é **derivada** da lista, não copiada: assim ela acompanha as mudanças
 * do repositório enquanto a tela está aberta — que é como o bloco de permissão
 * some sozinho quando alguém responde no desktop.
 */
@HiltViewModel
class SessionViewModel @Inject constructor(
    private val sessoesRepo: SessionRepository,
    handle: SavedStateHandle,
) : ViewModel() {

    private val id: String = checkNotNull(handle["id"]) { "rota de sessão sem id" }

    /**
     * Três estados, e não um `AgentSession?`.
     *
     * O `null` estava carregando dois significados incompatíveis — "ainda não
     * resolvi" e "não existe mais" —, e a tela reagia ao segundo. Como o valor
     * inicial de um `stateIn` é sempre o valor inicial, a tela lia "não existe
     * mais" na primeira composição e voltava antes de a sessão chegar: tocar numa
     * sessão não abria nada.
     *
     * Separar os casos é o conserto. Um `null` que significa duas coisas é um bug
     * esperando o momento; três estados nomeados não têm onde esconder.
     */
    private val _sessao = MutableStateFlow<EstadoDaSessao>(EstadoDaSessao.Carregando)

    /**
     * Três estados, e não um `AgentSession?`.
     *
     * O `null` carregava dois significados incompatíveis — "ainda não resolvi" e
     * "não existe mais" —, e a tela reagia ao segundo, voltando antes de a sessão
     * chegar: tocar numa sessão não abria nada.
     *
     * **O acumulador vive aqui, e não dentro do fluxo.** Um `scan` sob
     * `WhileSubscribed` recomeça do valor inicial quando os assinantes somem e
     * voltam — e o efeito era pior que o defeito original: sessão que encerrou,
     * aplicativo ao segundo plano por 5 segundos, e ao voltar a tela ficava em
     * `Carregando` para sempre, sem cabeçalho e sem botão de voltar.
     */
    val sessao: StateFlow<EstadoDaSessao> = _sessao.asStateFlow()

    init {
        viewModelScope.launch {
            sessoesRepo.sessions.collect { lista ->
                val encontrada = lista.firstOrNull { it.id == id }
                _sessao.value = when {
                    encontrada != null -> {
                        // Pedido novo apaga o cartão da resposta anterior. Sem
                        // isto ele reaparecia para uma permissão que o usuário não
                        // respondeu, com o horário errado.
                        if (encontrada.pendingPermission != null) {
                            _permissaoResolvida.value = null
                            _falhaDaAcao.value = null
                        }
                        EstadoDaSessao.Ativa(encontrada)
                    }
                    // Sumiu depois de ter existido: encerrou. Guarda a última
                    // conhecida para a tela continuar legível.
                    _sessao.value is EstadoDaSessao.Ativa ->
                        EstadoDaSessao.Encerrada((_sessao.value as EstadoDaSessao.Ativa).sessao)
                    // Já encerrada continua encerrada.
                    _sessao.value is EstadoDaSessao.Encerrada -> _sessao.value
                    // Ainda não apareceu em snapshot nenhum. Continua carregando,
                    // e **não** vira "encerrada" — é a distinção que faltava.
                    else -> EstadoDaSessao.Carregando
                }
            }
        }
    }

    private val _prompt = MutableStateFlow("")
    val prompt: StateFlow<String> = _prompt.asStateFlow()

    private val _permissaoResolvida = MutableStateFlow<PermissaoResolvida?>(null)
    val permissaoResolvida: StateFlow<PermissaoResolvida?> = _permissaoResolvida.asStateFlow()

    /**
     * Código da última ação que falhou, ou `null`.
     *
     * Existe pelo mesmo motivo que o do histórico: sem captura, responder uma
     * permissão com a conexão caída fecharia o aplicativo. E aqui é pior — é o
     * gesto que o produto existe para permitir.
     */
    private val _falhaDaAcao = MutableStateFlow<String?>(null)
    val falhaDaAcao: StateFlow<String?> = _falhaDaAcao.asStateFlow()

    fun mudarPrompt(texto: String) {
        _prompt.value = texto
    }

    fun enviarPrompt() {
        val texto = _prompt.value
        if (texto.isBlank()) return
        _prompt.value = ""
        viewModelScope.launch {
            try {
                sessoesRepo.enviarPrompt(id, texto)
                _falhaDaAcao.value = null
            } catch (erro: Exception) {
                // `Exception`, e não só `ErroDoProtocolo`: a decodificação da
                // página lança `SerializationException` se o servidor mudar um
                // campo, e aquela não é do protocolo — escaparia do `launch` e
                // derrubaria o processo.
                // Devolve o texto ao campo: o usuário escreveu aquilo, e perder o
                // que ele escreveu por causa de uma queda de rede é inaceitável.
                _prompt.value = texto
                _falhaDaAcao.value = codigoDe(erro)
            }
        }
    }

    fun responderPermissao(acao: PermissionAction) {
        val pedido = (_sessao.value as? EstadoDaSessao.Ativa)?.sessao?.pendingPermission ?: return
        // Registrado **antes** de a chamada voltar. O cartão precisa aparecer no
        // mesmo toque; esperar a confirmação faria o bloco piscar para vazio no
        // intervalo, que parece defeito.
        //
        // Guarda a ação, e não o texto dela: aqui não há `Context` nem locale, e
        // um rótulo montado neste ponto chegaria em português a um aparelho em
        // inglês. Quem traduz é a tela.
        _permissaoResolvida.value = PermissaoResolvida(
            acao = acao,
            horario = horaDoRelogio(System.currentTimeMillis()),
            emOutroDispositivo = false,
        )
        viewModelScope.launch {
            try {
                sessoesRepo.resolverPermissao(id, pedido.id, acao)
                _falhaDaAcao.value = null
            } catch (erro: Exception) {
                if (codigoDe(erro) == "permission_gone") {
                    // **Não é falha.** O protocolo diz que é situação normal:
                    // alguém respondeu antes, em outro lugar. O cartão fica, agora
                    // dizendo a verdade sobre quem respondeu.
                    _permissaoResolvida.value =
                        _permissaoResolvida.value?.copy(emOutroDispositivo = true)
                    _falhaDaAcao.value = null
                } else {
                    // Desfaz o cartão otimista: a permissão **não** foi
                    // respondida, e deixá-lo diria que o portão abriu quando ele
                    // continua fechado.
                    _permissaoResolvida.value = null
                    _falhaDaAcao.value = codigoDe(erro)
                }
            }
        }
    }
}

/**
 * Histórico paginado.
 *
 * Busca na primeira composição e depois só quando o rodapé aparece. Não escuta
 * nada: o protocolo não empurra histórico, e um `StateFlow` aqui sugeriria que
 * empurra.
 */
@HiltViewModel
class HistoryViewModel @Inject constructor(
    private val historicoRepo: HistoryRepository,
) : ViewModel() {

    private val _entradas = MutableStateFlow<List<HistoryEntry>>(emptyList())
    val entradas: StateFlow<List<HistoryEntry>> = _entradas.asStateFlow()

    private val _carregando = MutableStateFlow(false)
    val carregando: StateFlow<Boolean> = _carregando.asStateFlow()

    private val _noTeto = MutableStateFlow(false)
    val noTeto: StateFlow<Boolean> = _noTeto.asStateFlow()

    /**
     * Por que a última tentativa falhou, como **código** do protocolo.
     *
     * Código e não frase: a mensagem que o `ConnectionManager` carrega é literal
     * em pt-BR, e passá-la à tela levaria português para um aparelho em inglês.
     * Quem traduz é `strings.xml`.
     */
    private val _falha = MutableStateFlow<String?>(null)
    val falha: StateFlow<String?> = _falha.asStateFlow()

    private var cursor: HistoryCursor? = null
    private var acabou = false

    init {
        carregarMais()
    }

    fun carregarMais() {
        // A guarda dupla importa: o rodapé pode compor duas vezes seguidas
        // durante uma rolagem rápida, e sem ela a mesma página entraria duplicada.
        if (_carregando.value || acabou) return
        _carregando.value = true
        viewModelScope.launch {
            try {
                val pagina = historicoRepo.listar(before = cursor)
                _entradas.value = _entradas.value + pagina.entries
                cursor = pagina.nextCursor
                acabou = pagina.nextCursor == null
                _noTeto.value = pagina.atCeiling
                _falha.value = null
            } catch (erro: Exception) {
                // Sem esta captura o aplicativo **fecha**: uma exceção que escapa
                // de `viewModelScope.launch` derruba o processo. Era o que
                // acontecia ao abrir o Histórico sem aparelho pareado — não há
                // canal, `requisitar` lança `desconectado`, e ninguém pegava.
                _falha.value = codigoDe(erro)
            } finally {
                // No `finally` de propósito: deixar `carregando` verdadeiro após
                // uma falha travaria qualquer tentativa seguinte, porque a guarda
                // no topo desta função devolve cedo.
                _carregando.value = false
            }
        }
    }

    /** Nova tentativa depois de uma falha — o rodapé some e o pedido refaz. */
    fun tentarNovamente() {
        _falha.value = null
        acabou = false
        carregarMais()
    }
}

@HiltViewModel
class SettingsViewModel @Inject constructor(
    private val preferencias: PreferenciasEmMemoria,
    private val pareamento: PairingRepository,
    private val atualizador: Atualizador,
    sessoesRepo: SessionRepository,
) : ViewModel() {

    val desktop: StateFlow<PairedDesktop?> = pareamento.desktop
    val conexao: StateFlow<ConnectionState> = sessoesRepo.connection
    val tema: StateFlow<ThemeMode> = preferencias.tema
    val avisarPermissao: StateFlow<Boolean> = preferencias.avisarPermissao
    val avisarConclusao: StateFlow<Boolean> = preferencias.avisarConclusao

    /**
     * Vem do [Atualizador], que é `@Singleton`, e não de um estado próprio daqui.
     *
     * Deliberado: um download de trinta e quatro megabytes não pode ser interrompido
     * porque a pessoa saiu de Ajustes. O estado pertence a quem faz o trabalho, e a
     * tela apenas o observa — o inverso do que aconteceu com o pareamento, onde o
     * estado global era o defeito. A diferença é o tempo de vida real da operação:
     * um pareamento morre com a tela, um download precisa sobreviver a ela.
     */
    val atualizacao: StateFlow<EstadoDaAtualizacao> = atualizador.estado

    fun definirTema(modo: ThemeMode) = preferencias.definirTema(modo)
    fun definirAvisoDePermissao(ligado: Boolean) = preferencias.definirAvisoDePermissao(ligado)
    fun definirAvisoDeConclusao(ligado: Boolean) = preferencias.definirAvisoDeConclusao(ligado)

    fun esquecerDesktop() {
        viewModelScope.launch { pareamento.esquecerDesktop() }
    }

    fun verificarAtualizacao() {
        viewModelScope.launch { atualizador.verificar() }
    }

    fun baixarAtualizacao() {
        viewModelScope.launch { atualizador.baixar() }
    }

    fun instalarAtualizacao() = atualizador.instalar()

    fun autorizarInstalacao() = atualizador.abrirAutorizacao()
}

/**
 * A tela de pareamento.
 *
 * Ela é a única que fala com o [ConnectionManager] sem passar por repositório: o
 * pareamento não é leitura nem escrita de domínio, é a negociação que cria o
 * domínio. Inventar um `PairingRepository.parear()` só para respeitar a simetria
 * poria uma camada entre a tela e a única coisa que ela precisa fazer.
 *
 * O nome do aparelho vai como `Build.MODEL` — "Pixel 8". O servidor corta em 64
 * caracteres e troca vazio por "Celular", então não há o que validar aqui.
 */
/**
 * Onde a tentativa de pareamento está, **do ponto de vista da tela**.
 *
 * Vive no ViewModel e morre com ele. Sair da tela e voltar dá tentativa nova por
 * construção; uma falha não bloqueia a leitura seguinte; ir a segundo plano não
 * deixa nada preso. Era exatamente isso que faltava quando o estado morava no
 * gerente de conexão, que é `@Singleton`.
 */
sealed interface EstadoDePareamento {
    data object Ocioso : EstadoDePareamento
    data object EmAndamento : EstadoDePareamento
    data object Concluido : EstadoDePareamento
    data class Falhou(val motivo: String) : EstadoDePareamento
}

@HiltViewModel
class PairViewModel @Inject constructor(
    private val pareador: Pareador,
) : ViewModel() {

    private val _estado = MutableStateFlow<EstadoDePareamento>(EstadoDePareamento.Ocioso)
    val estado: StateFlow<EstadoDePareamento> = _estado.asStateFlow()

    private val _falhaDeLeitura = MutableStateFlow<FalhaDeLeitura?>(null)
    val falhaDeLeitura: StateFlow<FalhaDeLeitura?> = _falhaDeLeitura.asStateFlow()

    /** Nome do computador, assim que o QR é lido. Alimenta "Conectando a X…". */
    private val _nomeDoDesktop = MutableStateFlow<String?>(null)
    val nomeDoDesktop: StateFlow<String?> = _nomeDoDesktop.asStateFlow()

    /**
     * Chamado a cada QR que a câmera lê — inclusive o mesmo, várias vezes.
     *
     * A guarda contra repetição só vale **enquanto a tentativa acontece**. Depois
     * de uma falha o estado volta a ocioso, senão apontar a câmera de novo não
     * faria nada — que era o defeito.
     */
    fun aoLerQr(texto: String) {
        if (_estado.value == EstadoDePareamento.EmAndamento) return
        if (_estado.value == EstadoDePareamento.Concluido) return
        lerConvite(texto)
            .onSuccess { convite ->
                _falhaDeLeitura.value = null
                _nomeDoDesktop.value = convite.nomeDaMaquina.ifBlank { null }
                _estado.value = EstadoDePareamento.EmAndamento
                viewModelScope.launch {
                    val resultado = runCatching {
                        pareador.parear(convite, android.os.Build.MODEL)
                    }.getOrElse { ResultadoDoPareamento.Falhou("Não foi possível parear") }
                    _estado.value = when (resultado) {
                        is ResultadoDoPareamento.Pareado -> EstadoDePareamento.Concluido
                        is ResultadoDoPareamento.Falhou ->
                            EstadoDePareamento.Falhou(resultado.motivo)
                    }
                }
            }
            .onFailure { erro ->
                // `NaoEUmConvite` não vira mensagem: a câmera aponta para o mundo,
                // e avisar a cada QR de embalagem seria ruído. Os outros são
                // convites de verdade que falharam.
                val motivo = (erro as? LeituraDeConviteFalhou)?.motivo
                if (motivo != null && motivo !is FalhaDeLeitura.NaoEUmConvite) {
                    _falhaDeLeitura.value = motivo
                }
            }
    }

    /** Depois de uma falha, o usuário pode apontar a câmera de novo. */
    fun tentarDeNovo() {
        _estado.value = EstadoDePareamento.Ocioso
        _falhaDeLeitura.value = null
    }
}
