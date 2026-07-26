package lume.ai.ui

import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import lume.ai.data.PreferenciasEmMemoria
import lume.ai.data.repo.HistoryRepository
import lume.ai.data.repo.PairingRepository
import lume.ai.data.repo.SessionRepository
import lume.ai.domain.AgentSession
import lume.ai.domain.ConnectionState
import lume.ai.domain.HistoryCursor
import lume.ai.domain.HistoryEntry
import lume.ai.domain.PairedDesktop
import lume.ai.domain.PermissionAction
import lume.ai.ui.session.PermissaoResolvida
import lume.ai.ui.theme.ThemeMode
import javax.inject.Inject

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
) : ViewModel() {

    val tema: StateFlow<ThemeMode> = preferencias.tema

    val desktop: StateFlow<PairedDesktop?> = pareamento.desktop

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

    val sessao: StateFlow<AgentSession?> = sessoesRepo.sessions
        .map { lista -> lista.firstOrNull { it.id == id } }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    private val _prompt = MutableStateFlow("")
    val prompt: StateFlow<String> = _prompt.asStateFlow()

    private val _permissaoResolvida = MutableStateFlow<PermissaoResolvida?>(null)
    val permissaoResolvida: StateFlow<PermissaoResolvida?> = _permissaoResolvida.asStateFlow()

    fun mudarPrompt(texto: String) {
        _prompt.value = texto
    }

    fun enviarPrompt() {
        val texto = _prompt.value
        if (texto.isBlank()) return
        _prompt.value = ""
        viewModelScope.launch { sessoesRepo.enviarPrompt(id, texto) }
    }

    fun responderPermissao(acao: PermissionAction) {
        val pedido = sessao.value?.pendingPermission ?: return
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
        viewModelScope.launch { sessoesRepo.resolverPermissao(id, pedido.id, acao) }
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
            val pagina = historicoRepo.listar(before = cursor)
            _entradas.value = _entradas.value + pagina.entries
            cursor = pagina.nextCursor
            acabou = pagina.nextCursor == null
            _noTeto.value = pagina.atCeiling
            _carregando.value = false
        }
    }
}

@HiltViewModel
class SettingsViewModel @Inject constructor(
    private val preferencias: PreferenciasEmMemoria,
    private val pareamento: PairingRepository,
    sessoesRepo: SessionRepository,
) : ViewModel() {

    val desktop: StateFlow<PairedDesktop?> = pareamento.desktop
    val conexao: StateFlow<ConnectionState> = sessoesRepo.connection
    val tema: StateFlow<ThemeMode> = preferencias.tema
    val avisarPermissao: StateFlow<Boolean> = preferencias.avisarPermissao
    val avisarConclusao: StateFlow<Boolean> = preferencias.avisarConclusao

    fun definirTema(modo: ThemeMode) = preferencias.definirTema(modo)
    fun definirAvisoDePermissao(ligado: Boolean) = preferencias.definirAvisoDePermissao(ligado)
    fun definirAvisoDeConclusao(ligado: Boolean) = preferencias.definirAvisoDeConclusao(ligado)

    fun esquecerDesktop() {
        viewModelScope.launch { pareamento.esquecerDesktop() }
    }
}
