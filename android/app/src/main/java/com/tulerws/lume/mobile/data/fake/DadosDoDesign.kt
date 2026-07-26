package com.tulerws.lume.mobile.data.fake

import com.tulerws.lume.mobile.domain.AccessMode
import com.tulerws.lume.mobile.domain.ActivityStatus
import com.tulerws.lume.mobile.domain.AgentKind
import com.tulerws.lume.mobile.domain.AgentSession
import com.tulerws.lume.mobile.domain.HistoryEntry
import com.tulerws.lume.mobile.domain.HistoryEvent
import com.tulerws.lume.mobile.domain.PairedDesktop
import com.tulerws.lume.mobile.domain.PermissionAction
import com.tulerws.lume.mobile.domain.PermissionProfile
import com.tulerws.lume.mobile.domain.PermissionRequest
import com.tulerws.lume.mobile.domain.RiskLevel
import com.tulerws.lume.mobile.domain.SessionActivity
import com.tulerws.lume.mobile.domain.SessionSource
import com.tulerws.lume.mobile.domain.SessionStatus
import java.time.LocalDate
import java.time.LocalTime
import java.time.ZoneId

/**
 * Os dados que aparecem no arquivo de design, e nada além.
 *
 * Nome de agente, projeto, comando, horário e texto de atividade foram copiados
 * de `Lume Mobile.dc.html`. Inventar dados diferentes tornaria impossível
 * comparar a tela com o design lado a lado, que é como este trabalho é conferido.
 *
 * **O texto aqui fica em português e não vai para `strings.xml`.** Não é interface:
 * é carga útil de protocolo. O backend envia `agentLabel`, `event` e `summary`
 * como texto pronto em pt-BR, e um dado falso que se traduzisse sozinho estaria
 * simulando um servidor que não existe.
 *
 * Os instantes são relativos ao momento em que o objeto é criado. Fixá-los em
 * epoch absoluto faria "há 2 min" virar "há 8 meses" alguns meses depois — e o
 * defeito apareceria como se fosse do formatador de tempo.
 */
internal class DadosDoDesign(private val agora: Long = System.currentTimeMillis()) {

    private fun minutosAtras(m: Long) = agora - m * 60_000L
    private fun horasAtras(h: Long) = agora - h * 3_600_000L

    /** Converte um horário de relógio de hoje em epoch, para as atividades. */
    private fun hoje(hora: Int, minuto: Int): Long =
        LocalDate.now()
            .atTime(LocalTime.of(hora, minuto))
            .atZone(ZoneId.systemDefault())
            .toInstant()
            .toEpochMilli()

    private fun ontem(hora: Int, minuto: Int): Long = hoje(hora, minuto) - 86_400_000L

    val desktop = PairedDesktop(nome = "marcos-desktop", pareadoEm = "12 jul")

    /**
     * Perfil de quem o Lume pode responder: Claude e Codex abertos pelo Lume.
     *
     * `OpenSource` entra na lista de propósito, para exercitar o filtro de
     * `acoesRespondiveis()` — ela existe no protocolo e **não** pode virar botão.
     */
    private val podeResponder = PermissionProfile(
        mode = AccessMode.WorkspaceWrite,
        label = "Confirma alterações e comandos",
        approvalPolicy = "on-request",
        canRespondFromLume = true,
        availableActions = listOf(
            PermissionAction.AllowOnce,
            PermissionAction.AllowSession,
            PermissionAction.Deny,
            PermissionAction.OpenSource,
        ),
    )

    /**
     * Perfil de quem o Lume só observa — Gemini, Codex externo e páginas web.
     *
     * Existe nos dados falsos por ser o caso que a interface erra mais fácil: com
     * `canRespondFromLume = false`, nenhum botão pode ser desenhado, mesmo que
     * `availableActions` esteja cheia.
     */
    private val somenteObservacao = PermissionProfile(
        mode = AccessMode.Custom,
        label = "Gerenciada na origem",
        approvalPolicy = "external",
        canRespondFromLume = false,
        availableActions = listOf(PermissionAction.AllowOnce, PermissionAction.Deny),
    )

    private val permissaoDeRiscoAlto = PermissionRequest(
        id = "perm-1",
        kind = "command",
        summary = "Executar comando",
        resource = "rm -rf build/",
        risk = RiskLevel.High,
        requestedAt = "14:24",
    )

    private val claudeLume = AgentSession(
        id = "s-claude-lume",
        agent = AgentKind.Claude,
        agentLabel = "Claude",
        project = "lume",
        source = SessionSource.Cli,
        status = SessionStatus.PermissionRequired,
        statusLabel = "Aguardando permissão",
        startedAt = "14:18",
        updatedAt = minutosAtras(2),
        workingDirectory = "~/projects/lume",
        permissionProfile = podeResponder,
        pendingPermission = permissaoDeRiscoAlto,
        activities = listOf(
            SessionActivity(
                id = "a-1",
                kind = "read",
                title = "Leu ANDROID.md e MOBILE-DESIGN.md",
                status = ActivityStatus.Running,
                createdAt = hoje(14, 21),
            ),
            SessionActivity(
                id = "a-2",
                kind = "command",
                title = "Executou",
                detail = "pnpm build",
                status = ActivityStatus.Completed,
                createdAt = hoje(14, 23),
            ),
        ),
    )

    private val claudePipeline = AgentSession(
        id = "s-claude-pipeline",
        agent = AgentKind.Claude,
        agentLabel = "Claude",
        project = "pipeline-fiscal",
        source = SessionSource.Cli,
        status = SessionStatus.WaitingForInput,
        statusLabel = "Aguardando sua resposta",
        startedAt = "14:10",
        updatedAt = minutosAtras(4),
        workingDirectory = "~/projects/pipeline-fiscal",
        permissionProfile = podeResponder,
        lastResponse = "Terminei o mapeamento das notas. Quer que eu siga para a apuração?",
        activities = listOf(
            SessionActivity(
                id = "a-3",
                kind = "message",
                title = "Terminei o mapeamento das notas. Quer que eu siga para a apuração?",
                status = ActivityStatus.Waiting,
                createdAt = hoje(14, 20),
            ),
        ),
    )

    private val codexGateway = AgentSession(
        id = "s-codex-gateway",
        agent = AgentKind.Codex,
        agentLabel = "Codex",
        project = "api-gateway",
        source = SessionSource.Desktop,
        status = SessionStatus.Running,
        statusLabel = "Executando",
        startedAt = "14:12",
        updatedAt = minutosAtras(8),
        workingDirectory = "~/projects/api-gateway",
        permissionProfile = podeResponder,
        activities = listOf(
            SessionActivity(
                id = "a-4",
                kind = "command",
                title = "Executou",
                detail = "go test ./internal/gateway/...",
                status = ActivityStatus.Completed,
                createdAt = hoje(14, 26),
            ),
            SessionActivity(
                id = "a-5",
                kind = "message",
                title = "18 testes passaram. Estou ajustando o limite de conexões " +
                    "simultâneas no proxy antes de abrir o PR.",
                status = ActivityStatus.Running,
                createdAt = hoje(14, 28),
            ),
            SessionActivity(
                id = "a-6",
                kind = "edit",
                title = "Editou",
                detail = "internal/gateway/pool.go",
                status = ActivityStatus.Running,
                createdAt = hoje(14, 30),
            ),
        ),
    )

    private val geminiSite = AgentSession(
        id = "s-gemini-site",
        agent = AgentKind.Gemini,
        agentLabel = "Gemini",
        project = "site-institucional",
        source = SessionSource.Web,
        sourceApp = "Chromium",
        status = SessionStatus.Running,
        statusLabel = "Executando",
        startedAt = "13:59",
        updatedAt = minutosAtras(11),
        permissionProfile = somenteObservacao,
        activities = listOf(
            SessionActivity(
                id = "a-7",
                kind = "message",
                title = "Reescrevendo a seção de contato.",
                status = ActivityStatus.Running,
                createdAt = hoje(14, 5),
            ),
        ),
    )

    private val claudeAndroid = AgentSession(
        id = "s-claude-android",
        agent = AgentKind.Claude,
        agentLabel = "Claude",
        project = "lume-android",
        source = SessionSource.Cli,
        status = SessionStatus.Completed,
        statusLabel = "Finalizado",
        startedAt = "13:02",
        updatedAt = minutosAtras(26),
        workingDirectory = "~/projects/lume/android",
        permissionProfile = podeResponder,
        lastResponse = "Telas de sessões e histórico prontas, com tema claro e escuro.",
        activities = listOf(
            SessionActivity(
                id = "a-8",
                kind = "message",
                title = "Telas de sessões e histórico prontas, com tema claro e escuro.",
                status = ActivityStatus.Completed,
                createdAt = hoje(13, 12),
            ),
        ),
    )

    private val codexTerraform = AgentSession(
        id = "s-codex-terraform",
        agent = AgentKind.Codex,
        agentLabel = "Codex",
        project = "infra-terraform",
        source = SessionSource.Desktop,
        status = SessionStatus.Failed,
        statusLabel = "Falhou",
        startedAt = "12:40",
        updatedAt = horasAtras(1),
        workingDirectory = "~/projects/infra-terraform",
        permissionProfile = podeResponder,
        activities = listOf(
            SessionActivity(
                id = "a-9",
                kind = "command",
                title = "Executou",
                detail = "terraform plan",
                status = ActivityStatus.Failed,
                createdAt = hoje(13, 5),
            ),
        ),
    )

    /** As seis linhas do design, na ordem em que ele as mostra. */
    val sessoes = listOf(
        claudeLume,
        claudePipeline,
        codexGateway,
        geminiSite,
        claudeAndroid,
        codexTerraform,
    )

    /**
     * As sete entradas de histórico do design.
     *
     * Sem comando, sem caminho, sem payload — é o que o `PRIVACY.md` garante e o
     * que faz esta tela ser o dado de menor risco do produto. Os dados falsos
     * respeitam a garantia: se um `resource` aparecesse aqui, o exemplo estaria
     * ensinando o formato errado.
     */
    val historico = listOf(
        entrada("h-1", "s-claude-lume", "Claude", "lume", HistoryEvent.PermissionAllowed, hoje(14, 24)),
        entrada("h-2", "s-codex-gateway", "Codex", "api-gateway", HistoryEvent.PermissionAllowed, hoje(13, 58)),
        entrada("h-3", "s-claude-android", "Claude", "lume-android", HistoryEvent.Completed, hoje(13, 12)),
        entrada("h-4", "s-gemini-site", "Gemini", "site-institucional", HistoryEvent.Failed, hoje(11, 47)),
        entrada("h-5", "s-gemini-site", "Gemini", "site-institucional", HistoryEvent.PermissionDenied, hoje(11, 40)),
        entrada("h-6", "s-codex-terraform", "Codex", "infra-terraform", HistoryEvent.Failed, ontem(22, 3)),
        entrada("h-7", "s-claude-pipeline", "Claude", "pipeline-fiscal", HistoryEvent.Completed, ontem(19, 31)),
    )

    /**
     * Monta uma entrada com `event` e `summary` **nos campos certos**.
     *
     * `event` é código, `summary` é a frase que o backend monta em pt-BR
     * (`state.rs:739` produz o par `("permission_denied", "Permissão recusada")`).
     * Um dado falso que trocasse os dois esconderia na interface exatamente o
     * defeito que ele deveria expor.
     */
    private fun entrada(
        id: String,
        sessao: String,
        agente: String,
        projeto: String,
        evento: HistoryEvent,
        instante: Long,
    ) = HistoryEntry(
        id = id,
        sessionId = sessao,
        agentLabel = agente,
        project = projeto,
        event = evento,
        summary = when (evento) {
            HistoryEvent.PermissionAllowed -> "Permissão concedida"
            HistoryEvent.PermissionDenied -> "Permissão recusada"
            HistoryEvent.Completed -> "Tarefa finalizada"
            HistoryEvent.Failed -> "Tarefa encerrada com erro"
            HistoryEvent.Desconhecido -> ""
        },
        createdAt = instante,
    )
}
