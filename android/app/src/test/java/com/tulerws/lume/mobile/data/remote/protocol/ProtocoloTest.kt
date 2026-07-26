package com.tulerws.lume.mobile.data.remote.protocol

import com.tulerws.lume.mobile.domain.AgentKind
import com.tulerws.lume.mobile.domain.HistoryEvent
import com.tulerws.lume.mobile.domain.HistoryEntry
import com.tulerws.lume.mobile.domain.PermissionAction
import com.tulerws.lume.mobile.domain.RiskLevel
import com.tulerws.lume.mobile.domain.SessionStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * O protocolo v1, lido do lado do celular.
 *
 * Os JSON abaixo são transcrições do que `remote_server.rs` serializa, não
 * invenções: é o teste que pega divergência entre as duas implementações antes do
 * usuário, como `docs/ANDROID.md` pede.
 *
 * Quando o Rust mudar um nome de campo, é aqui que aparece — e não numa tela em
 * branco no aparelho.
 */
class ProtocoloTest {

    private val sessaoJson = """
        {
          "id": "s-1",
          "agent": "claude",
          "agentLabel": "Claude",
          "project": "lume",
          "source": "cli",
          "sourceApp": null,
          "status": "permission_required",
          "statusLabel": "Aguardando permissão",
          "startedAt": "14:18",
          "updatedAt": 1753400000000,
          "processId": 4294967295,
          "nativeSessionId": null,
          "workingDirectory": "~/projects/lume",
          "permissionProfile": {
            "mode": "workspace_write",
            "label": "Confirma alterações e comandos",
            "approvalPolicy": "on-request",
            "approvalsReviewer": null,
            "canRespondFromLume": true,
            "availableActions": ["allow_once", "allow_session", "deny", "open_source"]
          },
          "pendingPermission": {
            "id": "perm-1",
            "kind": "command",
            "summary": "Executar comando",
            "resource": "rm -rf build/",
            "risk": "high",
            "requestedAt": "14:24"
          },
          "lastResponse": null,
          "results": [],
          "activities": [
            { "id": "a-1", "kind": "command", "title": "Executou", "detail": "pnpm build",
              "status": "completed", "createdAt": 1753399000000, "files": [] }
          ]
        }
    """.trimIndent()

    @Test
    fun `snapshot desserializa uma sessao inteira`() {
        val mensagem = lerMensagem("""{"type":"sessions.snapshot","payload":{"sessions":[$sessaoJson]}}""")
        val sessao = (mensagem as MensagemDoServidor.Snapshot).payload.sessions.single()

        assertEquals("s-1", sessao.id)
        assertEquals(AgentKind.Claude, sessao.agent)
        assertEquals(SessionStatus.PermissionRequired, sessao.status)
        assertEquals(RiskLevel.High, sessao.pendingPermission?.risk)
        assertEquals(
            listOf(
                PermissionAction.AllowOnce,
                PermissionAction.AllowSession,
                PermissionAction.Deny,
                PermissionAction.OpenSource,
            ),
            sessao.permissionProfile.availableActions,
        )
        // `u32` no Rust. Se este campo fosse `Int` aqui, o valor máximo derrubaria
        // a leitura do snapshot inteiro por causa de um dado que nem é exibido.
        assertEquals(4_294_967_295L, sessao.processId)
    }

    @Test
    fun `campo desconhecido do servidor nao derruba a leitura`() {
        // Compatibilidade para a frente: uma v2 pode acrescentar campo, e um
        // aparelho v1 precisa continuar funcionando. É o que a versão do protocolo
        // promete no aperto de mão.
        val comExtra = sessaoJson.replace("\"id\": \"s-1\",", "\"id\": \"s-1\", \"campoNovo\": 42,")
        val mensagem = lerMensagem("""{"type":"sessions.snapshot","payload":{"sessions":[$comExtra]}}""")
        assertEquals("s-1", (mensagem as MensagemDoServidor.Snapshot).payload.sessions.single().id)
    }

    @Test
    fun `risco desconhecido vira alto`() {
        // Retaguarda deliberada: o desktop sabe de algo que este cliente não sabe,
        // e tratar isso como risco baixo tiraria a confirmação de dois toques
        // justamente do caso que ninguém previu.
        val comRiscoNovo = sessaoJson.replace("\"risk\": \"high\"", "\"risk\": \"catastrofico\"")
        val mensagem = lerMensagem("""{"type":"sessions.snapshot","payload":{"sessions":[$comRiscoNovo]}}""")
        val sessao = (mensagem as MensagemDoServidor.Snapshot).payload.sessions.single()
        assertEquals(RiskLevel.High, sessao.pendingPermission?.risk)
    }

    @Test
    fun `ready traz a versao do protocolo`() {
        val mensagem = lerMensagem(
            """{"type":"ready","payload":{"protocolVersion":1,"appVersion":"0.5.0",""" +
                """"hostname":"marcos-desktop","serverTime":1753400000000}}""",
        )
        val pronto = (mensagem as MensagemDoServidor.Pronto).payload
        assertEquals(VERSAO_DO_PROTOCOLO, pronto.protocolVersion)
        assertEquals("marcos-desktop", pronto.hostname)
    }

    @Test
    fun `erro carrega codigo e mensagem`() {
        val mensagem = lerMensagem(
            """{"type":"error","id":"req-1","payload":{"code":"permission_gone","message":"já respondida"}}""",
        )
        val falha = mensagem as MensagemDoServidor.Falha
        assertEquals("req-1", falha.id)
        assertEquals("permission_gone", falha.payload.code)
    }

    @Test
    fun `tipo desconhecido nao lanca`() {
        // Uma v2 pode acrescentar mensagens. Derrubar a conexão por causa delas
        // seria transformar compatibilidade para a frente em queda.
        val mensagem = lerMensagem("""{"type":"notify","payload":{"kind":"completed"}}""")
        assertTrue(mensagem is MensagemDoServidor.Desconhecida)
    }

    @Test
    fun `pedido de permissao serializa como o Rust espera`() {
        val texto = JsonDoProtocolo.encodeToString(
            EnvelopeDeSaida.serializer(ResolvePermission.serializer()),
            EnvelopeDeSaida(
                tipo = "permission.resolve",
                id = "req-1",
                payload = ResolvePermission("s-1", "perm-1", PermissionAction.AllowOnce),
            ),
        )
        // `type` e não `tipo`; `allow_once` e não `AllowOnce`. Os dois são o que o
        // `serde` do outro lado espera ler.
        assertTrue(texto.contains(""""type":"permission.resolve""""))
        assertTrue(texto.contains(""""sessionId":"s-1""""))
        assertTrue(texto.contains(""""action":"allow_once""""))
    }

    // ─── Delta ───────────────────────────────────────────────────────────────

    private fun sessao(id: String, status: SessionStatus) =
        lerMensagem("""{"type":"sessions.snapshot","payload":{"sessions":[${sessaoJson.replace("\"id\": \"s-1\"", "\"id\": \"$id\"").replace("\"status\": \"permission_required\"", "\"status\": \"${status.name.lowercase()}\"")}]}}""")
            .let { (it as MensagemDoServidor.Snapshot).payload.sessions.single() }

    @Test
    fun `delta aplica ordem do servidor`() {
        val cache = listOf(sessao("a", SessionStatus.Running), sessao("b", SessionStatus.Running))
        val resultado = aplicarDelta(cache, SessionsDelta(order = listOf("b", "a")))
        // A ordem é regra do Rust. O cliente aplica, não recalcula.
        assertEquals(listOf("b", "a"), resultado.map { it.id })
    }

    @Test
    fun `delta remove e mescla antes de reordenar`() {
        val cache = listOf(sessao("a", SessionStatus.Running), sessao("b", SessionStatus.Running))
        val nova = sessao("c", SessionStatus.Completed)
        val resultado = aplicarDelta(
            cache,
            SessionsDelta(updated = listOf(nova), removed = listOf("a"), order = listOf("c", "b")),
        )
        assertEquals(listOf("c", "b"), resultado.map { it.id })
    }

    @Test
    fun `delta para sessao que nao existe localmente apenas acrescenta`() {
        // O caso que `docs/ANDROID.md` nomeia entre os testes obrigatórios.
        val resultado = aplicarDelta(emptyList(), SessionsDelta(updated = listOf(sessao("z", SessionStatus.Running)), order = listOf("z")))
        assertEquals(listOf("z"), resultado.map { it.id })
    }

    @Test
    fun `sessao fora de order e descartada`() {
        // `order` é a lista completa. Estar fora dela significa não existir mais;
        // manter o resto no fim acumularia sessões fantasma que nenhuma mensagem
        // futura removeria.
        val cache = listOf(sessao("a", SessionStatus.Running), sessao("b", SessionStatus.Running))
        val resultado = aplicarDelta(cache, SessionsDelta(order = listOf("a")))
        assertEquals(listOf("a"), resultado.map { it.id })
    }

    @Test
    fun `order vazio preserva a ordem atual`() {
        // Um delta só de conteúdo não deve zerar a lista.
        val cache = listOf(sessao("a", SessionStatus.Running), sessao("b", SessionStatus.Running))
        val resultado = aplicarDelta(cache, SessionsDelta(removed = listOf("a")))
        assertEquals(listOf("b"), resultado.map { it.id })
    }

    // ─── Histórico ───────────────────────────────────────────────────────────

    @Test
    fun `evento de historico e codigo, nao frase`() {
        val entrada = JsonDoProtocolo.decodeFromString(
            HistoryEntry.serializer(),
            """{"id":"h-1","sessionId":"s-1","agentLabel":"Claude","project":"lume",""" +
                """"event":"permission_allowed","summary":"Permissão concedida","createdAt":1753400000000}""",
        )
        assertEquals(HistoryEvent.PermissionAllowed, entrada.event)
        assertEquals("Permissão concedida", entrada.summary)
    }

    @Test
    fun `evento de historico desconhecido nao derruba a pagina`() {
        val entrada = JsonDoProtocolo.decodeFromString(
            HistoryEntry.serializer(),
            """{"id":"h-2","sessionId":"s-1","agentLabel":"Claude","project":"lume",""" +
                """"event":"algo_novo","summary":"Algo novo","createdAt":1753400000000}""",
        )
        assertEquals(HistoryEvent.Desconhecido, entrada.event)
    }

    @Test
    fun `envelope sem payload de tipo conhecido e violacao`() {
        val erro = runCatching { lerMensagem("""{"type":"ready"}""") }.exceptionOrNull()
        assertTrue(erro is IllegalArgumentException)
    }

    @Test
    fun `envelope sem type e violacao`() {
        val erro = runCatching { lerMensagem("""{"payload":{}}""") }.exceptionOrNull()
        assertTrue(erro is IllegalArgumentException)
    }

    @Test
    fun `id ausente em mensagem iniciada pelo servidor`() {
        val mensagem = lerMensagem("""{"type":"sessions.delta","payload":{"updated":[],"removed":[],"order":[]}}""")
        assertTrue(mensagem is MensagemDoServidor.Delta)
        assertNull((lerMensagem("""{"type":"result","payload":{"ok":true}}""") as MensagemDoServidor.Resultado).id)
    }
}
