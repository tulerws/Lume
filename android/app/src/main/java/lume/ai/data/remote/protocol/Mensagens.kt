package lume.ai.data.remote.protocol

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonPrimitive
import lume.ai.domain.AgentSession
import lume.ai.domain.HistoryCursor
import lume.ai.domain.HistoryPage
import lume.ai.domain.PermissionAction

/**
 * O protocolo v1, do lado do celular.
 *
 * Espelha as structs de `src-tauri/src/remote_server.rs`. O envelope é
 * `{ type, id?, payload }`, e a serialização segue a do backend: **camelCase**,
 * igual ao que o `serde` já emite para o webview.
 *
 * ## Por que a leitura é em dois passos
 *
 * O formato de `payload` depende de `type`, e o `type` só é conhecido depois de
 * ler o JSON. A alternativa idiomática seria um serializador polimórfico com
 * discriminador de classe, mas ele exige que o servidor emita o discriminador
 * **dentro** do payload — e o nosso está fora, no envelope. Ler o objeto, olhar o
 * `type` e então decodificar o `payload` é o que corresponde ao formato de fio
 * real, em vez de dobrar o formato para caber numa conveniência da biblioteca.
 */

/** Versão que este cliente fala. Divergência é recusada no aperto de mão. */
const val VERSAO_DO_PROTOCOLO = 1

/** Subprotocolo negociado em `Sec-WebSocket-Protocol`. */
const val SUBPROTOCOLO = "lume.v1"

/**
 * `ignoreUnknownKeys` é deliberado, e é o que permite ao desktop ganhar campo
 * novo sem derrubar aparelho antigo — a compatibilidade para a frente que a
 * versão do protocolo promete. `explicitNulls = false` evita mandar `"campo":null`
 * onde o Rust espera ausência.
 */
val JsonDoProtocolo: Json = Json {
    ignoreUnknownKeys = true
    explicitNulls = false
    encodeDefaults = true
}

// ─── Celular → servidor ──────────────────────────────────────────────────────

@Serializable
data class EnvelopeDeSaida<T>(
    @SerialName("type") val tipo: String,
    val id: String? = null,
    val payload: T,
)

@Serializable
data class PairRegister(val deviceName: String, val platform: String)

@Serializable
data class ResolvePermission(
    val sessionId: String,
    val permissionId: String,
    val action: PermissionAction,
)

@Serializable
data class SubmitPrompt(val sessionId: String, val prompt: String)

/**
 * Pedido de página do histórico.
 *
 * **O servidor ainda não trata esta mensagem** — `remote_server.rs` despacha
 * `permission.resolve` e `prompt.submit`, e qualquer outro tipo cai em
 * `invalid_request`. Ela está aqui mesmo assim, e o cliente a envia de verdade,
 * por duas razões:
 *
 * · No dia em que o `history.list` entrar no Rust, a tela funciona sem tocar em
 *   Kotlin nenhum. O contrário — descobrir só então que o cliente não sabia pedir
 *   — adiaria o trabalho para o pior momento.
 * · O erro que volta hoje é conhecido e nomeado. `RemoteHistoryRepository` trata
 *   `invalid_request` como "este desktop ainda não oferece histórico", que é
 *   informação verdadeira, em vez de fingir uma lista vazia.
 *
 * `before` ausente pede a página mais recente.
 */
@Serializable
data class HistoryList(val limit: Int = 50, val before: HistoryCursor? = null)

// ─── Servidor → celular ──────────────────────────────────────────────────────

@Serializable
data class Ready(
    val protocolVersion: Int,
    val appVersion: String,
    val hostname: String,
    val serverTime: Long,
)

@Serializable
data class PairAccepted(val deviceId: String, val token: String)

@Serializable
data class SessionsSnapshot(val sessions: List<AgentSession>)

/**
 * `order` é a lista **completa** de identificadores na ordem de exibição.
 *
 * Ela existe porque a ordem é regra de negócio do Rust — prioridade de estado,
 * depois `updatedAt` decrescente — e o cliente não deve reescrevê-la. O snapshot
 * não traz o campo porque lá ele seria cópia do array que já vem ordenado.
 */
@Serializable
data class SessionsDelta(
    val updated: List<AgentSession> = emptyList(),
    val removed: List<String> = emptyList(),
    val order: List<String> = emptyList(),
)

/**
 * Resposta de `result` a um `history.list`.
 *
 * Espelha `HistoryPage` do domínio; existe como alias para que fique explícito
 * que o payload de `result` **depende da requisição** — o envelope não diz qual
 * forma esperar, quem sabe é o `id` que o cliente guardou.
 */
typealias PaginaDeHistorico = HistoryPage

@Serializable
data class ProtocolError(val code: String, val message: String)

/**
 * Uma mensagem já reconhecida.
 *
 * [Desconhecida] não é erro: é a compatibilidade para a frente funcionando. Uma
 * v2 pode acrescentar mensagens, e um aparelho v1 deve ignorá-las em silêncio em
 * vez de derrubar a conexão — que é o que aconteceria se isto fosse `else ->
 * throw`.
 */
sealed interface MensagemDoServidor {
    data class Pronto(val payload: Ready) : MensagemDoServidor
    data class Pareado(val id: String?, val payload: PairAccepted) : MensagemDoServidor
    data class Snapshot(val payload: SessionsSnapshot) : MensagemDoServidor
    data class Delta(val payload: SessionsDelta) : MensagemDoServidor
    data class Resultado(val id: String?, val payload: JsonObject?) : MensagemDoServidor
    data class Falha(val id: String?, val payload: ProtocolError) : MensagemDoServidor
    data class Desconhecida(val tipo: String) : MensagemDoServidor
}

/**
 * Lê uma mensagem do fio.
 *
 * Lança quando o texto não é JSON, ou quando o payload de um tipo **conhecido**
 * não tem a forma esperada — os dois são violação de protocolo, e engoli-los
 * deixaria o aplicativo parado sem explicação. Tipo desconhecido não lança.
 */
fun lerMensagem(texto: String): MensagemDoServidor {
    val objeto = JsonDoProtocolo.parseToJsonElement(texto) as? JsonObject
        ?: throw IllegalArgumentException("Envelope não é um objeto JSON")
    val tipo = objeto["type"]?.jsonPrimitive?.content
        ?: throw IllegalArgumentException("Envelope sem campo `type`")
    val id = (objeto["id"] as? kotlinx.serialization.json.JsonPrimitive)?.contentOrNull
    val payload = objeto["payload"]

    fun <T> carga(serializador: kotlinx.serialization.DeserializationStrategy<T>): T {
        val elemento = payload
            ?: throw IllegalArgumentException("Mensagem `$tipo` sem payload")
        return JsonDoProtocolo.decodeFromJsonElement(serializador, elemento)
    }

    return when (tipo) {
        "ready" -> MensagemDoServidor.Pronto(carga(Ready.serializer()))
        "pair.accepted" -> MensagemDoServidor.Pareado(id, carga(PairAccepted.serializer()))
        "sessions.snapshot" -> MensagemDoServidor.Snapshot(carga(SessionsSnapshot.serializer()))
        "sessions.delta" -> MensagemDoServidor.Delta(carga(SessionsDelta.serializer()))
        "result" -> MensagemDoServidor.Resultado(id, payload as? JsonObject)
        "error" -> MensagemDoServidor.Falha(id, carga(ProtocolError.serializer()))
        else -> MensagemDoServidor.Desconhecida(tipo)
    }
}

private val kotlinx.serialization.json.JsonPrimitive.contentOrNull: String?
    get() = if (this is kotlinx.serialization.json.JsonNull) null else content

/**
 * Aplica um delta sobre o cache.
 *
 * Três passos, nesta ordem: remove, mescla, reordena.
 *
 * A reordenação usa **apenas** `order`, e sessão ausente de `order` é descartada
 * — o servidor manda a lista completa, então estar fora dela significa não existir
 * mais. Manter o resto no fim "por segurança" acumularia sessões fantasma que
 * nenhuma mensagem futura removeria.
 *
 * `order` vazio é tratado como "não informado" e preserva a ordem atual, porque um
 * delta só de conteúdo não deve zerar a lista.
 */
fun aplicarDelta(atual: List<AgentSession>, delta: SessionsDelta): List<AgentSession> {
    val porId = atual.associateByTo(LinkedHashMap()) { it.id }
    delta.removed.forEach(porId::remove)
    delta.updated.forEach { porId[it.id] = it }
    if (delta.order.isEmpty()) return porId.values.toList()
    return delta.order.mapNotNull(porId::get)
}
