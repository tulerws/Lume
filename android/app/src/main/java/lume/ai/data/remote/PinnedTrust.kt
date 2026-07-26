package lume.ai.data.remote

import okhttp3.OkHttpClient
import java.security.MessageDigest
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSession
import javax.net.ssl.X509TrustManager

/**
 * Confiança fixada por fingerprint.
 *
 * ## O que o QR entrega, e o que ele não entrega
 *
 * `docs/ANDROID.md` sugere `HandshakeCertificates.Builder().addTrustedCertificate(cert)`,
 * "sem `X509TrustManager` escrito à mão". **Isso não é aplicável no primeiro
 * contato**: `addTrustedCertificate` exige o certificado inteiro, e o QR carrega
 * apenas o SHA-256 dele — 32 bytes, por orçamento de densidade
 * (`REMOTE-CONTROL.md`, *Orçamento de densidade*). Não há como tornar âncora de
 * confiança um certificado que ainda não se tem.
 *
 * A saída não é afrouxar nada. É mover a verificação para onde a informação
 * existe: o gestor de confiança recebe a cadeia apresentada e compara o SHA-256
 * do certificado **folha** com o fingerprint fixado. O critério é mais estrito
 * que o de uma âncora de confiança comum: nenhuma autoridade é aceita, e a única
 * chave que serve é aquela.
 *
 * Certificados extras na cadeia são ignorados, não recusados — o servidor sobe
 * com `with_single_cert` e não manda nenhum, e recusar por tamanho de cadeia
 * seria checar o que não importa: o TLS já exige posse da chave privada da folha,
 * e é a folha que o fingerprint identifica.
 *
 * ## Por que a checagem acontece duas vezes
 *
 * [TrustManagerFixado] recusa no aperto de mão; [verificadorDeFingerprint] recusa
 * de novo, na verificação de nome. Não é redundância desatenta:
 *
 * · O verificador de nome **precisa** ser substituído de qualquer jeito. O
 *   certificado do desktop é imutável e o IP dele não é — DHCP renova, o notebook
 *   sai do Wi-Fi para o Ethernet, sobe VPN. O `OkHostnameVerifier` padrão mataria
 *   o aperto de mão com `Hostname not verified` no primeiro desses eventos.
 * · Substituí-lo por algo que devolva `true` seria a falha exata que este desenho
 *   evita. Então ele devolve uma comparação real — a mesma —, e o caso negativo é
 *   teste obrigatório dos dois lados.
 *
 * O SAN do certificado segue existindo para mensagem de erro legível e
 * diagnóstico. Ele não decide nada: **a identidade é a chave**.
 */

/** SHA-256 do certificado em DER — a mesma conta que `remote_identity.rs` faz. */
fun fingerprintDe(certificado: X509Certificate): ByteArray =
    MessageDigest.getInstance("SHA-256").digest(certificado.encoded)

/**
 * Gestor de confiança de um certificado só.
 *
 * Ignora a cadeia e as autoridades do sistema de propósito: um certificado
 * autoassinado não tem cadeia para validar, e uma autoridade pública que
 * assinasse qualquer coisa não teria autoridade nenhuma sobre *este* desktop.
 */
internal class TrustManagerFixado(private val fingerprint: ByteArray) : X509TrustManager {

    override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {
        val folha = chain?.firstOrNull()
            ?: throw CertificateException("O servidor não apresentou certificado")
        if (!MessageDigest.isEqual(fingerprintDe(folha), fingerprint)) {
            // Mensagem sem o fingerprint recebido: ela chega ao usuário como
            // "não é o computador pareado", e imprimir os dois valores só ajudaria
            // quem estivesse tentando acertar um deles.
            throw CertificateException("O certificado apresentado não é o do desktop pareado")
        }
    }

    /**
     * O aplicativo nunca apresenta certificado: o servidor sobe com
     * `with_no_client_auth`. Um cliente que aceitasse ser desafiado aqui estaria
     * respondendo a uma pergunta que este protocolo não faz.
     */
    override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) =
        throw CertificateException("Este cliente não apresenta certificado")

    override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
}

/**
 * Verificador de nome que compara chave, não nome.
 *
 * Aceita quando o fingerprint bate **ainda que o hostname não conste do SAN** —
 * que é o caso normal, com IP variável — e recusa quando o fingerprint não bate,
 * ainda que o hostname confira. Os dois casos juntos são o que prova que a
 * decisão saiu do nome e foi para a chave.
 */
internal fun verificadorDeFingerprint(fingerprint: ByteArray) =
    javax.net.ssl.HostnameVerifier { _: String?, sessao: SSLSession ->
        // `peerCertificates` lança `SSLPeerUnverifiedException` quando não há
        // certificado. Capturar e devolver `false` deixa o caminho explícito:
        // deixar a exceção subir também falharia fechado, mas por acidente, e
        // acidente não é garantia.
        val folha = runCatching { sessao.peerCertificates.firstOrNull() }.getOrNull()
        if (folha !is X509Certificate) return@HostnameVerifier false
        MessageDigest.isEqual(fingerprintDe(folha), fingerprint)
    }

/**
 * Um cliente HTTP que só fala com **este** desktop.
 *
 * ## Os dois prazos, e por que são diferentes
 *
 * `readTimeout` vale **só até o upgrade**. Depois dele o OkHttp zera o tempo de
 * leitura do socket em `RealConnection.newWebSocketStreams()`, porque WebSocket
 * ocioso é normal. Sem um valor finito aqui, um servidor que completa o TLS e
 * nunca responde ao `GET /lume` seguraria a tentativa para sempre: `onFailure`
 * não dispararia, e o backoff de reconexão nunca começaria. O `connectTimeout`
 * padrão não cobre esse caso — ele para em TCP mais TLS.
 *
 * `callTimeout` **não** serve: ele limitaria a chamada inteira, e a chamada é o
 * WebSocket, que deve durar horas.
 *
 * ## O ping daqui não é o ping do documento
 *
 * `REMOTE-CONTROL.md` descreve "três pings sem resposta derrubam a conexão", e
 * isso é o **servidor** — `MAX_MISSED_PONGS = 3` em `remote_server.rs`. O
 * `pingInterval` abaixo é a deteção do lado do cliente, e o OkHttp derruba no
 * primeiro pong faltante, não no terceiro: são ~30s em vez de ~90s. Isso não
 * contraria a regra do servidor, porque não é a mesma regra — e detectar canal
 * morto mais cedo do lado que precisa reconectar é o comportamento desejável.
 *
 * O pong para o ping do servidor é automático no OkHttp, então a regra dos três
 * segue satisfeita do outro lado.
 */
fun clienteFixado(fingerprint: ByteArray): OkHttpClient {
    require(fingerprint.size == 32) {
        "Fingerprint precisa ter 32 bytes; veio com ${fingerprint.size}"
    }
    val gestor = TrustManagerFixado(fingerprint)
    val contexto = SSLContext.getInstance("TLS").apply {
        init(null, arrayOf<javax.net.ssl.TrustManager>(gestor), java.security.SecureRandom())
    }
    return OkHttpClient.Builder()
        .sslSocketFactory(contexto.socketFactory, gestor)
        .hostnameVerifier(verificadorDeFingerprint(fingerprint))
        .pingInterval(java.time.Duration.ofSeconds(30))
        .readTimeout(java.time.Duration.ofSeconds(15))
        .build()
}
