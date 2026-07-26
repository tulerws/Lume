package com.tulerws.lume.mobile.data.repo

import kotlinx.coroutines.flow.StateFlow
import com.tulerws.lume.mobile.domain.AgentSession
import com.tulerws.lume.mobile.domain.ConnectionState
import com.tulerws.lume.mobile.domain.HistoryCursor
import com.tulerws.lume.mobile.domain.HistoryPage
import com.tulerws.lume.mobile.domain.PairedDesktop
import com.tulerws.lume.mobile.domain.PermissionAction

/**
 * Contratos entre a interface e o mundo.
 *
 * Hoje o único implementador é falso. Estas assinaturas são, mesmo assim, as
 * definitivas — o cliente real implementa estas interfaces, e trocar um pelo
 * outro é trocar uma linha no módulo do Hilt. O risco de decidir a forma antes do
 * protocolo rodar está registrado: se ela sair errada, o custo aparece na
 * integração.
 *
 * `docs/ANDROID.md` prevê dois repositórios (`SessionRepository`,
 * `PairingRepository`). Aqui são três. O histórico ganhou o seu porque o contrato
 * dele é outro: sessões são um `StateFlow` que o servidor empurra, histórico é
 * `suspend` com paginação sob demanda, e o protocolo diz explicitamente que
 * histórico **nunca** é empurrado. Um tipo com as duas naturezas convidaria a
 * tratar histórico como estado vivo, que é justamente o que duplicaria tráfego.
 */

interface SessionRepository {

    /** Estado do canal. A tela de Sessões o mostra de forma permanente. */
    val connection: StateFlow<ConnectionState>

    /**
     * Sessões observadas, na ordem em que devem aparecer.
     *
     * **A ordem vem do servidor e não é recalculada aqui.** `AppState::sessions`
     * ordena por prioridade de estado e depois por `updatedAt` decrescente — quem
     * espera permissão flutua para o topo —, e essa regra é de negócio: ela decide
     * o que o usuário vê primeiro, que é a razão de ele ter aberto o aplicativo.
     *
     * No fio isso chega de duas formas, e as duas são obrigatórias:
     *
     * · `sessions.snapshot` entrega o array **já ordenado**; exibe-se como veio.
     * · `sessions.delta` traz `order`, a lista **completa** de identificadores na
     *   ordem de exibição; o cache é reordenado por ela.
     *
     * Reordenar em Kotlin — mesmo "do mesmo jeito" — cria duas regras que
     * coincidem hoje e divergem no dia em que um estado novo entrar no Rust. A
     * divergência apareceria justamente na linha que o usuário mais precisa ver
     * primeiro, e em silêncio. Ver `docs/REMOTE-CONTROL.md`.
     */
    val sessions: StateFlow<List<AgentSession>>

    /**
     * Responde um pedido de permissão.
     *
     * O cliente não é fonte de verdade: o backend revalida ação e sessão antes de
     * liberar o `Condvar`. Uma falha aqui é informação para a tela, não motivo
     * para a tela ter deixado de mostrar o botão.
     */
    suspend fun resolverPermissao(sessionId: String, permissionId: String, action: PermissionAction)

    /**
     * Envia um prompt.
     *
     * Recusa por sessão ocupada é estado esperado, não erro — e a tela já deve ter
     * impedido a chamada desabilitando o campo.
     */
    suspend fun enviarPrompt(sessionId: String, prompt: String)

    /** Nova tentativa de conexão, disparada pelo botão "Tentar de novo". */
    fun reconectar()
}

interface HistoryRepository {

    /**
     * Uma página, da mais recente para a mais antiga.
     *
     * @param before `null` pede a página mais recente.
     */
    suspend fun listar(limit: Int = 50, before: HistoryCursor? = null): HistoryPage
}

interface PairingRepository {

    /** O desktop pareado, ou `null` quando o aplicativo ainda não foi pareado. */
    val desktop: StateFlow<PairedDesktop?>

    /**
     * Apaga cache e credencial e volta o aplicativo à tela de pareamento.
     *
     * Destrutivo e sem volta: quem chama precisa ter confirmado antes.
     */
    suspend fun esquecerDesktop()
}
