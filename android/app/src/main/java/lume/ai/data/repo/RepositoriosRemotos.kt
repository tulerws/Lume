package lume.ai.data.repo

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.SharingStarted
import lume.ai.data.ConnectionManager
import lume.ai.data.ErroDoProtocolo
import lume.ai.domain.AgentSession
import lume.ai.domain.ConnectionState
import lume.ai.domain.HistoryCursor
import lume.ai.domain.HistoryPage
import lume.ai.domain.PairedDesktop
import lume.ai.domain.PermissionAction
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Os repositórios de verdade, sobre o [ConnectionManager].
 *
 * São finos de propósito: adaptam o dono da conexão às três interfaces que a
 * interface consome, e não guardam estado próprio. Estado duplicado entre o
 * gerente e um repositório seria a mesma verdade em dois lugares, com uma cópia
 * sempre atrasada.
 */

@Singleton
class RemoteSessionRepository @Inject constructor(
    private val conexao: ConnectionManager,
) : SessionRepository {

    override val connection: StateFlow<ConnectionState> = conexao.connection

    /**
     * A lista chega já ordenada e é exibida como veio.
     *
     * Não há `sortedBy` aqui, e não pode haver: a ordem é regra do Rust, entregue
     * pelo array do `sessions.snapshot` e pelo `order` do `sessions.delta`.
     */
    override val sessions: StateFlow<List<AgentSession>> = conexao.sessions

    override suspend fun resolverPermissao(
        sessionId: String,
        permissionId: String,
        action: PermissionAction,
    ) = conexao.resolverPermissao(sessionId, permissionId, action)

    override suspend fun enviarPrompt(sessionId: String, prompt: String) =
        conexao.enviarPrompt(sessionId, prompt)

    override fun reconectar() {
        // `conectar()` é idempotente e reinicia o backoff do zero. É o que
        // "Tentar de novo" precisa fazer: não esperar os 30 segundos do laço.
        conexao.desconectar()
        conexao.conectar()
    }
}

/**
 * O desktop pareado, visto do celular.
 *
 * Enquanto a credencial vier do `local.properties`, o "pareamento" é a
 * configuração de quem desenvolve — e o nome só existe depois do `ready`, porque
 * é o servidor que o informa. Antes disso não há o que mostrar, e `null` é a
 * resposta honesta.
 *
 * `pareadoEm` fica vazio pelo mesmo motivo: não houve pareamento com data. O
 * campo ganha valor quando o `pair.accepted` for guardado no Keystore.
 */
@Singleton
class RemotePairingRepository @Inject constructor(
    private val conexao: ConnectionManager,
) : PairingRepository {

    private val escopo = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    override val desktop: StateFlow<PairedDesktop?> = conexao.hostname
        .map { nome -> nome?.let { PairedDesktop(nome = it, pareadoEm = "") } }
        .stateIn(escopo, SharingStarted.Eagerly, null)

    override suspend fun esquecerDesktop() {
        // Hoje só derruba a conexão: a credencial vive no `BuildConfig`, e um
        // aplicativo não apaga o próprio build. Quando o token for gravado no
        // Keystore, é aqui que ele é apagado — e a assinatura já é a definitiva.
        conexao.desconectar()
    }
}

/**
 * Histórico, pedido ao desktop.
 *
 * O servidor **ainda não trata** `history.list`. Este repositório envia mesmo
 * assim e trata a recusa: `invalid_request` vira uma página vazia com
 * [HistoryPage.atCeiling] falso, que a tela mostra como estado vazio.
 *
 * A alternativa seria não enviar nada, e ela é pior por duas razões. A tela
 * ficaria indistinguível de um desktop que respondeu "não há nada", e o dia em
 * que o Rust ganhar a mensagem passaria despercebido — enquanto assim ela começa
 * a funcionar sozinha.
 */
@Singleton
class RemoteHistoryRepository @Inject constructor(
    private val conexao: ConnectionManager,
) : HistoryRepository {

    private val _indisponivel = MutableStateFlow(false)

    /** Se o desktop recusou a mensagem. A tela pode usar para explicar o vazio. */
    val indisponivel: StateFlow<Boolean> = _indisponivel

    override suspend fun listar(limit: Int, before: HistoryCursor?): HistoryPage = try {
        val pagina = conexao.listarHistorico(limit, before)
        _indisponivel.value = false
        pagina
    } catch (erro: ErroDoProtocolo) {
        // `invalid_request` aqui significa "este desktop não conhece a mensagem",
        // e não "o pedido estava torto" — o pedido é montado por código tipado.
        if (erro.codigo == "invalid_request") {
            _indisponivel.value = true
            HistoryPage(entries = emptyList(), nextCursor = null, atCeiling = false)
        } else {
            throw erro
        }
    }
}
