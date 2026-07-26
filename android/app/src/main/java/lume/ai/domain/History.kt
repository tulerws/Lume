package lume.ai.domain

import androidx.compose.runtime.Immutable
import kotlinx.serialization.KSerializer
import kotlinx.serialization.Serializable
import kotlinx.serialization.descriptors.PrimitiveKind
import kotlinx.serialization.descriptors.PrimitiveSerialDescriptor
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder

/**
 * Histórico sanitizado, espelho de `HistoryEntry` em `src-tauri/src/domain.rs`.
 *
 * É o dado de menor risco do produto: evento, resumo, agente, projeto e horário —
 * **sem comando, sem caminho, sem payload**. A garantia está no `PRIVACY.md`, e é
 * o motivo de esta tela poder existir no celular sem discussão.
 */
/**
 * O que aconteceu. **Código, não frase.**
 *
 * `event` e `summary` são fáceis de trocar: `event` é o código em snake_case
 * (`state.rs:739` produz o par `("permission_denied", "Permissão recusada")`), e
 * `summary` é a frase em pt-BR. Renderizar `event` direto na tela mostraria
 * `permission_allowed` ao usuário.
 *
 * O conjunto é fechado em quatro valores (`src/lib/domain.ts:96`), então ele vira
 * enum e o rótulo exibido sai de `strings.xml` — mesma decisão que já vale para
 * `SessionStatus`, e pelos mesmos dois motivos: comprimento previsível numa linha
 * de altura fixa, e inglês sem portar tabela de tradução.
 *
 * [Desconhecido] existe porque o campo é `String` no Rust: um evento novo lá não
 * pode derrubar a leitura da página inteira aqui.
 */
@Serializable(with = HistoryEventSerializer::class)
enum class HistoryEvent { Completed, Failed, PermissionAllowed, PermissionDenied, Desconhecido }

internal object HistoryEventSerializer : KSerializer<HistoryEvent> {
    override val descriptor: SerialDescriptor =
        PrimitiveSerialDescriptor("HistoryEvent", PrimitiveKind.STRING)

    override fun serialize(encoder: Encoder, value: HistoryEvent) = encoder.encodeString(
        when (value) {
            HistoryEvent.Completed -> "completed"
            HistoryEvent.Failed -> "failed"
            HistoryEvent.PermissionAllowed -> "permission_allowed"
            HistoryEvent.PermissionDenied -> "permission_denied"
            HistoryEvent.Desconhecido -> "unknown"
        },
    )

    override fun deserialize(decoder: Decoder): HistoryEvent = when (decoder.decodeString()) {
        "completed" -> HistoryEvent.Completed
        "failed" -> HistoryEvent.Failed
        "permission_allowed" -> HistoryEvent.PermissionAllowed
        "permission_denied" -> HistoryEvent.PermissionDenied
        else -> HistoryEvent.Desconhecido
    }
}

@Immutable
@Serializable
data class HistoryEntry(
    val id: String,
    val sessionId: String,
    val agentLabel: String,
    val project: String,
    /** Código do evento. O rótulo exibido vem de `strings.xml`, nunca daqui. */
    val event: HistoryEvent,
    /**
     * Frase em pt-BR montada pelo backend — "Permissão concedida", "Tarefa
     * finalizada". Fica no espelho porque o protocolo a envia, e serve de
     * retaguarda para [HistoryEvent.Desconhecido], que é o único caso em que a
     * interface não tem rótulo próprio para mostrar.
     */
    val summary: String,
    val createdAt: Long,
)

/**
 * Cursor de paginação.
 *
 * O desempate por [id] não é zelo: este projeto já teve colisão de identificadores
 * no mesmo milissegundo, e um cursor só com [createdAt] pularia ou repetiria
 * entradas quando isso acontecesse.
 */
@Immutable
@Serializable
data class HistoryCursor(val createdAt: Long, val id: String)

/**
 * Uma página de histórico.
 *
 * [atCeiling] distingue duas situações que pareceriam iguais na tela: o histórico
 * acabou de verdade, ou o servidor chegou ao teto de 200 registros que o desktop
 * também tem. Sem esse campo, a interface teria que escolher entre sugerir que o
 * histórico terminou ali — mentira — ou oferecer um botão de carregar mais que
 * não carregaria nada.
 */
@Immutable
@Serializable
data class HistoryPage(
    val entries: List<HistoryEntry>,
    val nextCursor: HistoryCursor?,
    val atCeiling: Boolean,
)
