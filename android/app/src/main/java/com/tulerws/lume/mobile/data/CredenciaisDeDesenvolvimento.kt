package com.tulerws.lume.mobile.data

import com.tulerws.lume.mobile.BuildConfig
import com.tulerws.lume.mobile.data.remote.Credencial
import com.tulerws.lume.mobile.data.remote.Endereco
import javax.inject.Inject
import javax.inject.Singleton

/** Tudo que é preciso para abrir um canal: onde, com que credencial, contra qual chave. */
data class Alvo(
    val endereco: Endereco,
    val credencial: Credencial,
    val fingerprint: ByteArray,
)

/**
 * O alvo vindo do `local.properties`, espelho do `LUME_REMOTE_DEV`.
 *
 * Existe pelo mesmo motivo que a variável do lado Rust: parear exige o QR, ler o
 * QR exige a câmera, e a câmera ainda não está neste cliente. O documento cria a
 * variável justamente para quebrar essa circularidade "sem inventar credencial
 * paralela" — o token de desenvolvimento vira uma linha da tabela `remote_devices`
 * e a autenticação continua sendo a definitiva.
 *
 * Em `release` os quatro campos são vazios e [alvo] devolve `null`. Não há
 * caminho por onde uma credencial de desenvolvimento chegue a um build publicado:
 * ela não está no `BuildConfig` daquela variante.
 *
 * **Isto some quando a câmera entrar.** O que fica é o [Alvo], que a leitura do
 * QR vai preencher do mesmo jeito.
 */
@Singleton
class CredenciaisDeDesenvolvimento @Inject constructor() {

    fun alvo(): Alvo? {
        val host = BuildConfig.DEV_HOST.trim()
        val token = BuildConfig.DEV_TOKEN.trim()
        val fingerprintEmHex = BuildConfig.DEV_FINGERPRINT.trim()
        val porta = BuildConfig.DEV_PORT.trim().toIntOrNull() ?: 43140

        if (host.isEmpty() || token.isEmpty() || fingerprintEmHex.isEmpty()) return null

        val fingerprint = deHex(fingerprintEmHex) ?: return null
        if (fingerprint.size != 32) return null

        return Alvo(
            endereco = Endereco(host, porta),
            credencial = Credencial.Token(token),
            fingerprint = fingerprint,
        )
    }

    /**
     * Hexadecimal porque é o que `sha256sum identity.der` imprime.
     *
     * O QR usa base64url; aqui a origem é a linha de comando de quem desenvolve, e
     * pedir uma conversão a mais só criaria uma chance a mais de errar em silêncio.
     * Entrada malformada devolve `null` e o aplicativo abre desconectado — melhor
     * do que conectar comparando contra bytes errados.
     */
    private fun deHex(texto: String): ByteArray? {
        if (texto.length % 2 != 0) return null
        return runCatching {
            ByteArray(texto.length / 2) { i ->
                texto.substring(i * 2, i * 2 + 2).toInt(16).toByte()
            }
        }.getOrNull()
    }
}
