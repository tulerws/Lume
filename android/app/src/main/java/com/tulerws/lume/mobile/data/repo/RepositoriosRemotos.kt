package com.tulerws.lume.mobile.data.repo

import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.SharingStarted
import com.tulerws.lume.mobile.data.ConnectionManager
import com.tulerws.lume.mobile.data.crypto.CredentialStore
import com.tulerws.lume.mobile.domain.AgentSession
import com.tulerws.lume.mobile.domain.ConnectionState
import com.tulerws.lume.mobile.domain.HistoryCursor
import com.tulerws.lume.mobile.domain.HistoryPage
import com.tulerws.lume.mobile.domain.PairedDesktop
import com.tulerws.lume.mobile.domain.PermissionAction
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
 * **"Pareado" é ter credencial guardada, não estar conectado agora.** A distinção
 * decide o que o aplicativo mostra: um celular pareado que abriu sem rede está
 * *desconectado* e deve esperar; um celular que nunca pareou não tem a que se
 * reconectar e precisa de outra coisa — ler um QR.
 *
 * Derivar isto do `hostname` do `ready`, como a versão anterior fazia, confundia
 * os dois: no instante seguinte a abrir o aplicativo, antes de a conexão subir,
 * um aparelho pareado se anunciava como não pareado.
 */
@Singleton
class RemotePairingRepository @Inject constructor(
    private val conexao: ConnectionManager,
    private val credencial: CredentialStore,
) : PairingRepository {

    private val escopo = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    /**
     * Deriva direto do fluxo da credencial.
     *
     * Não há recarga manual, e não há quem a dispare. Gravar no pareamento emite,
     * apagar ao esquecer emite. A versão anterior observava o **estado do
     * pareamento** para saber quando reler — uma notificação paralela ao dado, e
     * portanto uma chance a mais de esquecer.
     */
    override val desktop: StateFlow<PairedDesktop?> = credencial.credencial
        .map { guardada ->
            guardada?.let { PairedDesktop(nome = it.nomeDoDesktop, pareadoEm = it.pareadoEm) }
        }
        .stateIn(escopo, SharingStarted.Eagerly, null)

    /**
     * Esquece o aparelho de verdade.
     *
     * Derruba a conexão **e** apaga a credencial — blob e chave. Só desconectar
     * deixaria o aplicativo reconectando sozinho no próximo `onStart`, o que é o
     * oposto de esquecer.
     */
    override suspend fun esquecerDesktop() {
        conexao.desconectar()
        credencial.apagar()
    }
}

/**
 * Histórico, pedido ao desktop.
 *
 * **Não engole nada.** A versão anterior transformava `invalid_request` numa
 * página vazia e marcava um campo que não estava na interface e que ninguém lia —
 * então a tela mostrava "Nada registrado ainda", que é mentira: não é que não
 * haja nada, é que não deu para perguntar. O comentário do protocolo dizia
 * literalmente "em vez de fingir uma lista vazia", e o código fingia.
 *
 * Agora o erro sobe até quem sabe traduzi-lo em texto — o `HistoryViewModel`.
 */
@Singleton
class RemoteHistoryRepository @Inject constructor(
    private val conexao: ConnectionManager,
) : HistoryRepository {

    override suspend fun listar(limit: Int, before: HistoryCursor?): HistoryPage =
        conexao.listarHistorico(limit, before)
}
