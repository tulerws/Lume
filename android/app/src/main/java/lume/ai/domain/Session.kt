package lume.ai.domain

import androidx.compose.runtime.Immutable
import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.descriptors.PrimitiveKind
import kotlinx.serialization.descriptors.PrimitiveSerialDescriptor
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder

/**
 * Espelho de `src-tauri/src/domain.rs`.
 *
 * "Espelho" é literal, não aproximado: os nomes, os campos e a cardinalidade dos
 * enums acompanham o Rust. Campo que muda lá muda aqui, e a versão do protocolo é
 * o que avisa quando os dois divergiram.
 *
 * ## Por que a anotação de serialização vive aqui, e não numa camada de DTO
 *
 * `docs/ANDROID.md` decide isso: *"kotlinx.serialization espelha `serde`; o
 * backend já emite camelCase, então os nomes batem sem adaptador"*. Uma camada de
 * DTO só existiria para copiar campo a campo entre dois tipos idênticos, e cada
 * campo novo do protocolo passaria a exigir três edições em vez de uma — que é
 * exatamente como espelhos param de espelhar.
 *
 * O custo aceito: `domain/` conhece o `kotlinx.serialization`. Em troca, a
 * divergência entre Rust e Kotlin vira erro de desserialização em vez de campo
 * silenciosamente perdido no meio de um mapeamento.
 *
 * ## O que **não** está aqui
 *
 * `resultNotes`: notas de resultado são recurso de autoria, e autoria no celular
 * não é o problema que este aplicativo resolve (`docs/MOBILE-UI-DESIGN.md`).
 *
 * Datas: o Rust tem `startedAt: String` e `updatedAt: i64`, e essa assimetria é
 * dele — `startedAt` é rótulo já formatado, `updatedAt` é epoch em milissegundos.
 * Preservada como está, porque corrigi-la aqui esconderia a divergência em vez de
 * resolvê-la no lugar certo.
 */

@Serializable
enum class AgentKind {
    @SerialName("codex") Codex,
    @SerialName("claude") Claude,
    @SerialName("gemini") Gemini,
    @SerialName("unknown") Unknown,
}

@Serializable
enum class SessionSource {
    @SerialName("cli") Cli,
    @SerialName("vscode") Vscode,
    @SerialName("web") Web,
    @SerialName("desktop") Desktop,
}

@Serializable
enum class SessionStatus {
    @SerialName("running") Running,
    @SerialName("permission_required") PermissionRequired,
    @SerialName("waiting_for_input") WaitingForInput,
    @SerialName("completed") Completed,
    @SerialName("failed") Failed,
}

@Serializable
enum class AccessMode {
    @SerialName("full_access") FullAccess,
    @SerialName("workspace_write") WorkspaceWrite,
    @SerialName("read_only") ReadOnly,
    @SerialName("plan") Plan,
    @SerialName("custom") Custom,
}

@Serializable
enum class PermissionAction {
    @SerialName("allow_once") AllowOnce,
    @SerialName("allow_session") AllowSession,
    @SerialName("deny") Deny,
    @SerialName("open_source") OpenSource,
}

/**
 * Risco de um pedido de permissão.
 *
 * No Rust o campo é `String`, não enum — o conjunto fechado existe só por
 * convenção (`src/lib/domain.ts:37`). Por isso a leitura tem retaguarda, e a
 * retaguarda é **[High]**.
 *
 * A escolha não é arbitrária: um valor desconhecido significa que o desktop sabe
 * de algo que este aplicativo não sabe, e tratar o desconhecido como baixo risco
 * removeria a confirmação de dois toques justamente do caso que ninguém previu.
 * Errar para o lado do atrito é recuperável; errar para o lado da aprovação fácil
 * não é.
 */
@Serializable(with = RiskLevelSerializer::class)
enum class RiskLevel { Low, Medium, High }

internal object RiskLevelSerializer : KSerializer<RiskLevel> {
    override val descriptor: SerialDescriptor =
        PrimitiveSerialDescriptor("RiskLevel", PrimitiveKind.STRING)

    override fun serialize(encoder: Encoder, value: RiskLevel) =
        encoder.encodeString(value.name.lowercase())

    override fun deserialize(decoder: Decoder): RiskLevel = when (decoder.decodeString()) {
        "low" -> RiskLevel.Low
        "medium" -> RiskLevel.Medium
        else -> RiskLevel.High
    }
}

/**
 * Estado de uma linha de atividade.
 *
 * Também `String` no Rust; os quatro valores estão em `src/lib/domain.ts:67`. A
 * retaguarda aqui é [Running] — o estado neutro. Ao contrário do risco, esta
 * escolha não protege nada: ela só decide a cor de um ponto de 8dp, e o neutro é
 * o que menos afirma sobre o que não se sabe.
 */
@Serializable(with = ActivityStatusSerializer::class)
enum class ActivityStatus { Running, Completed, Failed, Waiting }

internal object ActivityStatusSerializer : KSerializer<ActivityStatus> {
    override val descriptor: SerialDescriptor =
        PrimitiveSerialDescriptor("ActivityStatus", PrimitiveKind.STRING)

    override fun serialize(encoder: Encoder, value: ActivityStatus) =
        encoder.encodeString(value.name.lowercase())

    override fun deserialize(decoder: Decoder): ActivityStatus = when (decoder.decodeString()) {
        "completed" -> ActivityStatus.Completed
        "failed" -> ActivityStatus.Failed
        "waiting" -> ActivityStatus.Waiting
        else -> ActivityStatus.Running
    }
}

@Immutable
@Serializable
data class PermissionProfile(
    val mode: AccessMode,
    val label: String,
    val approvalPolicy: String,
    val approvalsReviewer: String? = null,
    /**
     * Se falso, o celular **não desenha ação nenhuma** — só observa. É a mesma
     * regra que a interface do desktop respeita, e afrouxá-la aqui daria ao
     * celular poder que o desktop não tem.
     */
    val canRespondFromLume: Boolean,
    /**
     * As únicas ações que podem virar botão. A interface não inventa botão que
     * não esteja nesta lista, mesmo quando "faz sentido".
     */
    val availableActions: List<PermissionAction>,
)

@Immutable
@Serializable
data class PermissionRequest(
    val id: String,
    val kind: String,
    /** Título do bloco: "Executar comando", "Editar arquivo". */
    val summary: String,
    /** O que será executado ou tocado. Aparece em monoespaçada, antes dos botões. */
    val resource: String,
    val risk: RiskLevel,
    val requestedAt: String,
)

@Immutable
@Serializable
data class SessionResult(
    val id: String,
    val response: String,
    val createdAt: Long,
    val files: List<String> = emptyList(),
    val tests: List<String> = emptyList(),
)

@Immutable
@Serializable
data class SessionActivity(
    val id: String,
    val kind: String,
    /** Frase principal: "Executou", "Leu ANDROID.md", ou a fala do agente. */
    val title: String,
    /** Comando ou caminho, quando houver. Renderizado em monoespaçada. */
    val detail: String? = null,
    val status: ActivityStatus,
    val createdAt: Long,
    val files: List<String> = emptyList(),
)

@Immutable
@Serializable
data class AgentSession(
    val id: String,
    val agent: AgentKind,
    val agentLabel: String,
    val project: String,
    val source: SessionSource,
    val sourceApp: String? = null,
    val status: SessionStatus,
    /**
     * Rótulo em pt-BR vindo do backend. **A interface não o usa** para a pílula de
     * estado: é texto livre, de comprimento imprevisível ("Executando no VS Code"),
     * e a linha de sessão tem 72dp de altura fixa. O rótulo exibido vem de
     * [status], via `strings.xml`, o que também é o que dá inglês de graça.
     *
     * Fica no espelho porque o protocolo o envia; some daqui no dia em que o
     * protocolo parar de enviar.
     */
    val statusLabel: String,
    val startedAt: String,
    val updatedAt: Long,
    /**
     * `u32` no Rust, e `Long` aqui de propósito: `u32` não cabe em `Int`, e um
     * valor acima de `Int.MAX_VALUE` derrubaria a leitura do snapshot inteiro por
     * causa de um campo que esta interface sequer mostra.
     */
    val processId: Long? = null,
    val nativeSessionId: String? = null,
    val workingDirectory: String? = null,
    val permissionProfile: PermissionProfile,
    val pendingPermission: PermissionRequest? = null,
    val lastResponse: String? = null,
    val results: List<SessionResult> = emptyList(),
    val activities: List<SessionActivity> = emptyList(),
)

/**
 * Ações que a interface pode desenhar para este pedido.
 *
 * Concentra numa função a regra que `docs/REMOTE-CONTROL.md` enuncia duas vezes:
 * a ação precisa estar em `availableActions` **e** `canRespondFromLume` precisa
 * ser verdadeiro. Espalhar essa conjunção pelos componentes seria dar a cada um
 * a chance de esquecer metade dela.
 *
 * `OpenSource` cai fora sempre: abrir a origem é ação de máquina local, e o
 * servidor a recusa com `action_not_available`. Desenhar um botão que o servidor
 * recusa é ensinar o usuário a desconfiar da interface.
 */
fun AgentSession.acoesRespondiveis(): List<PermissionAction> =
    if (!permissionProfile.canRespondFromLume) {
        emptyList()
    } else {
        permissionProfile.availableActions.filter { it != PermissionAction.OpenSource }
    }

/**
 * Se a sessão aceita prompt agora.
 *
 * Mesma regra do desktop: sessão em execução ou esperando resposta de permissão
 * recusa prompt com `session_busy`. Isso é estado esperado, não erro — o campo
 * fica desabilitado **com o motivo escrito**, nunca mudo.
 */
fun AgentSession.aceitaPrompt(): Boolean =
    status != SessionStatus.Running && status != SessionStatus.PermissionRequired

/**
 * Se enviar prompt daqui abre uma janela de terminal na máquina do usuário.
 *
 * `submit_prompt` tem três caminhos e eles não são equivalentes vistos de longe.
 * Claude e Gemini são retomados por `launcher::launch`, que **abre um terminal**
 * — com o usuário longe do computador. Codex aberto pelo Lume passa pelo App
 * Server, e sessão web pelo Companion do Chromium; nesses dois não há efeito
 * visível na máquina.
 *
 * O aviso aparece só nos dois primeiros. Avisar sempre treinaria o usuário a
 * ignorar o aviso, que é o oposto do objetivo.
 */
fun AgentSession.abreTerminalNaMaquina(): Boolean =
    agent == AgentKind.Claude || agent == AgentKind.Gemini
