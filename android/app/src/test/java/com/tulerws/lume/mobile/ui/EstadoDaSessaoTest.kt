package com.tulerws.lume.mobile.ui

import androidx.lifecycle.SavedStateHandle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import com.tulerws.lume.mobile.data.repo.SessionRepository
import com.tulerws.lume.mobile.domain.AccessMode
import com.tulerws.lume.mobile.domain.AgentKind
import com.tulerws.lume.mobile.domain.AgentSession
import com.tulerws.lume.mobile.domain.ConnectionState
import com.tulerws.lume.mobile.domain.PermissionAction
import com.tulerws.lume.mobile.domain.PermissionProfile
import com.tulerws.lume.mobile.domain.SessionSource
import com.tulerws.lume.mobile.domain.SessionStatus
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

/**
 * A regressão de tocar numa sessão e não ir a lugar nenhum.
 *
 * O defeito, visto no Galaxy M13: `sessao` era um `AgentSession?` cujo valor
 * inicial de `stateIn` era `null`, e a tela tratava `null` como "a sessão sumiu"
 * e voltava. Como o valor inicial é sempre emitido primeiro, a volta acontecia
 * **antes** de a sessão chegar. Tocar numa sessão navegava e retornava no mesmo
 * quadro.
 *
 * O primeiro teste abaixo é o que teria pego isso. Os outros guardam a distinção
 * que o conserto introduziu — três estados nomeados em vez de um `null` com dois
 * significados.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class EstadoDaSessaoTest {

    @Before
    fun antes() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun depois() = Dispatchers.resetMain()

    private val perfil = PermissionProfile(
        mode = AccessMode.WorkspaceWrite,
        label = "",
        approvalPolicy = "",
        canRespondFromLume = true,
        availableActions = listOf(PermissionAction.AllowOnce),
    )

    private fun sessaoDe(id: String) = AgentSession(
        id = id,
        agent = AgentKind.Claude,
        agentLabel = "Claude",
        project = "lume",
        source = SessionSource.Cli,
        status = SessionStatus.Running,
        statusLabel = "Executando",
        startedAt = "14:00",
        updatedAt = 0,
        permissionProfile = perfil,
    )

    /** Repositório de mentira: só o que o ViewModel lê. */
    private class RepositorioFalso(
        private val fluxo: MutableStateFlow<List<AgentSession>>,
    ) : SessionRepository {
        override val connection: StateFlow<ConnectionState> =
            MutableStateFlow(ConnectionState.Conectado)
        override val sessions: StateFlow<List<AgentSession>> = fluxo
        override suspend fun resolverPermissao(s: String, p: String, a: PermissionAction) = Unit
        override suspend fun enviarPrompt(sessionId: String, prompt: String) = Unit
        override fun reconectar() = Unit
    }

    private fun viewModel(fluxo: MutableStateFlow<List<AgentSession>>) =
        SessionViewModel(RepositorioFalso(fluxo), SavedStateHandle(mapOf("id" to "s-1")))

    @Test
    fun `antes do primeiro snapshot o estado e carregando, nunca encerrada`() = runTest {
        // A regressão. Se este estado voltar a ser "encerrada" — ou `null` tratado
        // como tal —, a tela volta sozinha e tocar numa sessão não abre nada.
        val vm = viewModel(MutableStateFlow(emptyList()))
        assertEquals(EstadoDaSessao.Carregando, vm.sessao.first())
    }

    @Test
    fun `lista sem a sessao ainda e carregando`() = runTest {
        // Um snapshot que não contém esta sessão pode significar apenas que ela
        // ainda não foi ingerida pelo desktop. Só vira "encerrada" depois de ter
        // existido.
        val vm = viewModel(MutableStateFlow(listOf(sessaoDe("outra"))))
        assertEquals(EstadoDaSessao.Carregando, vm.sessao.first())
    }

    @Test
    fun `sessao presente fica ativa`() = runTest {
        val vm = viewModel(MutableStateFlow(listOf(sessaoDe("s-1"))))
        val estado = vm.sessao.first()
        assertTrue(estado is EstadoDaSessao.Ativa)
        assertEquals("s-1", (estado as EstadoDaSessao.Ativa).sessao.id)
    }

    @Test
    fun `sessao que some depois de existir vira encerrada com a ultima conhecida`() = runTest {
        val fluxo = MutableStateFlow(listOf(sessaoDe("s-1")))
        val vm = viewModel(fluxo)
        assertTrue(vm.sessao.first() is EstadoDaSessao.Ativa)

        fluxo.value = emptyList()

        val estado = vm.sessao.first()
        assertTrue(estado is EstadoDaSessao.Encerrada)
        // O conteúdo continua legível: é o que o usuário estava lendo.
        assertEquals("s-1", (estado as EstadoDaSessao.Encerrada).ultimaConhecida.id)
    }

    @Test
    fun `encerrada sobrevive a reassinatura`() = runTest {
        // A regressão do segundo defeito. O acumulador vivia dentro de um `scan`
        // sob `WhileSubscribed(5s)`: quando ninguém assinava por 5 segundos o
        // fluxo era cancelado, e ao voltar recomeçava de `Carregando` — a tela
        // ficava presa num `Box` vazio, sem cabeçalho e sem saída.
        //
        // Agora o acumulador é campo do ViewModel. Coletar, largar e coletar de
        // novo não pode fazer o estado regredir.
        val fluxo = MutableStateFlow(listOf(sessaoDe("s-1")))
        val vm = viewModel(fluxo)
        assertTrue(vm.sessao.first() is EstadoDaSessao.Ativa)

        fluxo.value = emptyList()
        assertTrue(vm.sessao.first() is EstadoDaSessao.Encerrada)

        // Simula o ciclo de assinatura da tela: ninguém coletando, e depois de
        // novo. Com o `scan` sob `WhileSubscribed` isto voltava a `Carregando`.
        repeat(3) { assertTrue(vm.sessao.first() is EstadoDaSessao.Encerrada) }
        assertTrue(vm.sessao.value is EstadoDaSessao.Encerrada)
    }

    @Test
    fun `pedido novo apaga o cartao da resposta anterior`() = runTest {
        // Sem isto o cartão "Permissão concedida · 14:32" reaparecia para uma
        // permissão que o usuário não respondeu, com o horário errado.
        val comPedido = sessaoDe("s-1").copy(
            pendingPermission = com.tulerws.lume.mobile.domain.PermissionRequest(
                id = "perm-2",
                kind = "command",
                summary = "Executar comando",
                resource = "ls",
                risk = com.tulerws.lume.mobile.domain.RiskLevel.Low,
                requestedAt = "14:40",
            ),
        )
        val fluxo = MutableStateFlow(listOf(sessaoDe("s-1")))
        val vm = viewModel(fluxo)
        vm.sessao.first()
        fluxo.value = listOf(comPedido)
        vm.sessao.first()
        assertEquals(null, vm.permissaoResolvida.value)
    }

    @Test
    fun `encerrada nao volta a carregando`() = runTest {
        // Uma vez encerrada, um snapshot vazio seguinte não pode fazer a tela
        // regredir para o estado de abertura.
        val fluxo = MutableStateFlow(listOf(sessaoDe("s-1")))
        val vm = viewModel(fluxo)
        vm.sessao.first()
        fluxo.value = emptyList()
        vm.sessao.first()
        fluxo.value = listOf(sessaoDe("outra"))

        assertTrue(vm.sessao.first() is EstadoDaSessao.Encerrada)
    }
}
