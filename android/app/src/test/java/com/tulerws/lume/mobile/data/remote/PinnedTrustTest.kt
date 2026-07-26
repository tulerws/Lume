package com.tulerws.lume.mobile.data.remote

import okhttp3.tls.HeldCertificate
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.security.MessageDigest
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import javax.net.ssl.SSLSession

/**
 * O caso negativo do pinning.
 *
 * `docs/ANDROID.md` chama estes testes de obrigatórios, e diz por quê: *"sem ele,
 * o pinning pode estar desligado sem ninguém perceber"*. Um verificador que
 * devolvesse `true` incondicionalmente passaria em todo teste positivo — só o
 * negativo o denuncia.
 *
 * Rodam na JVM, sem aparelho e sem servidor: `TrustManagerFixado`,
 * `verificadorDeFingerprint` e `fingerprintDe` só dependem de `java.security`.
 */
class PinnedTrustTest {

    private val doDesktop = HeldCertificate.Builder()
        .commonName("lume.local")
        .addSubjectAlternativeName("lume.local")
        .build()

    private val deUmImpostor = HeldCertificate.Builder()
        .commonName("lume.local")
        .addSubjectAlternativeName("lume.local")
        .build()

    private val certificadoCerto: X509Certificate = doDesktop.certificate
    private val certificadoErrado: X509Certificate = deUmImpostor.certificate
    private val fingerprintCerto = fingerprintDe(certificadoCerto)

    @Test
    fun `fingerprint e o sha256 do certificado em DER`() {
        // A mesma conta de `remote_identity.rs`: SHA-256 sobre o DER inteiro.
        // Se este teste quebrar, o pinning nunca vai casar com o desktop.
        val esperado = MessageDigest.getInstance("SHA-256").digest(certificadoCerto.encoded)
        assertArrayEquals(esperado, fingerprintCerto)
        assertTrue(fingerprintCerto.size == 32)
    }

    /**
     * O erro mais fácil de cometer, e o que não dá sintoma.
     *
     * `cert.encoded` e `cert.publicKey.encoded` têm os dois 32 bytes depois do
     * SHA-256. Trocar um pelo outro compila, passa em qualquer teste positivo, e
     * chega ao usuário como "não conecta" — porque a comparação simplesmente
     * nunca bate. `docs/ANDROID.md` chama isto de teste obrigatório desde a
     * correção do desenho de confiança.
     */
    @Test
    fun `o fingerprint e do certificado, nao da chave publica`() {
        val daChavePublica = MessageDigest.getInstance("SHA-256")
            .digest(certificadoCerto.publicKey.encoded)
        assertEquals(32, daChavePublica.size)
        assertFalse(fingerprintCerto.contentEquals(daChavePublica))
    }

    /**
     * Um certificado que uma cadeia pública validaria.
     *
     * É o caso que separa "fixar chave" de "confiar em autoridade": o gestor não
     * aceita autoridade nenhuma, então uma cadeia impecável assinada por uma CA
     * é recusada igual. Um `TrustManager` de corpo vazio, ou um que delegasse ao
     * padrão do sistema, deixaria este passar.
     */
    @Test(expected = CertificateException::class)
    fun `o gestor recusa certificado assinado por autoridade`() {
        val autoridade = HeldCertificate.Builder()
            .certificateAuthority(0)
            .commonName("Autoridade de Teste")
            .build()
        val assinado = HeldCertificate.Builder()
            .commonName("lume.local")
            .addSubjectAlternativeName("lume.local")
            .signedBy(autoridade)
            .build()
        TrustManagerFixado(fingerprintCerto).checkServerTrusted(
            arrayOf(assinado.certificate, autoridade.certificate),
            "ECDHE_ECDSA",
        )
    }

    @Test
    fun `dois certificados diferentes tem fingerprints diferentes`() {
        // Guarda contra o teste acima passar por acidente — dois certificados com
        // o mesmo CN e o mesmo SAN precisam continuar distinguíveis pela chave.
        assertFalse(fingerprintCerto.contentEquals(fingerprintDe(certificadoErrado)))
    }

    @Test
    fun `o gestor aceita o certificado fixado`() {
        TrustManagerFixado(fingerprintCerto)
            .checkServerTrusted(arrayOf(certificadoCerto), "ECDHE_ECDSA")
    }

    @Test(expected = CertificateException::class)
    fun `o gestor recusa um certificado diferente`() {
        // O caso que importa: outro certificado, válido em si, com o mesmo nome.
        TrustManagerFixado(fingerprintCerto)
            .checkServerTrusted(arrayOf(certificadoErrado), "ECDHE_ECDSA")
    }

    @Test(expected = CertificateException::class)
    fun `o gestor recusa cadeia vazia`() {
        TrustManagerFixado(fingerprintCerto).checkServerTrusted(emptyArray(), "ECDHE_ECDSA")
    }

    @Test(expected = CertificateException::class)
    fun `o gestor recusa cadeia nula`() {
        TrustManagerFixado(fingerprintCerto).checkServerTrusted(null, "ECDHE_ECDSA")
    }

    @Test(expected = CertificateException::class)
    fun `o gestor nunca aceita ser desafiado como cliente`() {
        // O servidor sobe com `with_no_client_auth`; responder a este desafio
        // seria responder a uma pergunta que o protocolo não faz.
        TrustManagerFixado(fingerprintCerto).checkClientTrusted(arrayOf(certificadoCerto), "EC")
    }

    @Test
    fun `o verificador aceita hostname fora do SAN quando o fingerprint bate`() {
        // Este é o caso **normal**: o certificado é imutável, o IP não é, e
        // `192.168.0.14` nunca vai constar do SAN. Um verificador preso ao nome
        // mataria o aperto de mão aqui.
        val verificador = verificadorDeFingerprint(fingerprintCerto)
        assertTrue(verificador.verify("192.168.0.14", sessaoCom(certificadoCerto)))
    }

    @Test
    fun `o verificador recusa quando o fingerprint nao bate mesmo com hostname certo`() {
        // O par do teste acima. Só os dois juntos provam que a decisão saiu do
        // nome e foi para a chave: o primeiro sozinho passaria com um verificador
        // que devolvesse `true`.
        val verificador = verificadorDeFingerprint(fingerprintCerto)
        assertFalse(verificador.verify("lume.local", sessaoCom(certificadoErrado)))
    }

    @Test
    fun `o verificador recusa quando nao ha certificado`() {
        val verificador = verificadorDeFingerprint(fingerprintCerto)
        assertFalse(verificador.verify("lume.local", sessaoCom()))
    }

    @Test(expected = IllegalArgumentException::class)
    fun `fingerprint com tamanho errado e recusado na construcao`() {
        // 32 bytes é o que 43 caracteres de base64url decodificam. Tamanho
        // diferente significa QR corrompido, e falhar aqui é melhor do que
        // conectar comparando bytes contra nada.
        clienteFixado(ByteArray(31))
    }

    /** `SSLSession` de mentira: o verificador só olha `peerCertificates`. */
    private fun sessaoCom(vararg certificados: X509Certificate): SSLSession {
        val sessao = java.lang.reflect.Proxy.newProxyInstance(
            SSLSession::class.java.classLoader,
            arrayOf(SSLSession::class.java),
        ) { _, metodo, _ ->
            when (metodo.name) {
                "getPeerCertificates" -> if (certificados.isEmpty()) {
                    throw javax.net.ssl.SSLPeerUnverifiedException("sem certificado")
                } else {
                    certificados
                }

                else -> null
            }
        }
        return sessao as SSLSession
    }
}
