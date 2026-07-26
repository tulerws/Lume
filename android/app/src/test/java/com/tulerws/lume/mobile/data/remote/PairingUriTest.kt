package com.tulerws.lume.mobile.data.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.Base64

/**
 * Os cinco casos que `docs/ANDROID.md` nomeia como obrigatórios — URI válida,
 * versão desconhecida, campo ausente, fingerprint malformado e lista de
 * candidatos vazia — mais os detalhes que o `REMOTE-CONTROL.md` avisa que decidem
 * se a leitura funciona ou falha.
 *
 * Rodam na JVM porque o parser não usa `android.net.Uri`. Foi por isso que ele
 * não usa.
 */
class PairingUriTest {

    private val fingerprint = ByteArray(32) { it.toByte() }
    private val fingerprintEmB64: String =
        Base64.getUrlEncoder().withoutPadding().encodeToString(fingerprint)
    private val codigo: String =
        Base64.getUrlEncoder().withoutPadding().encodeToString(ByteArray(32) { (it + 7).toByte() })

    private fun uri(
        v: String = "1",
        f: String = fingerprintEmB64,
        c: String = codigo,
        p: String = "43140",
        h: String = "192.168.0.14,2001:db8::1",
        n: String = "marcos-desktop",
    ) = "lume://pair?v=$v&f=$f&c=$c&p=$p&h=$h&n=$n"

    private fun motivo(texto: String): FalhaDeLeitura =
        (lerConvite(texto).exceptionOrNull() as LeituraDeConviteFalhou).motivo

    @Test
    fun `uri valida`() {
        val convite = lerConvite(uri()).getOrThrow()
        assertTrue(convite.fingerprint.contentEquals(fingerprint))
        assertEquals(codigo, convite.codigo)
        assertEquals(43140, convite.porta)
        assertEquals(listOf("192.168.0.14", "2001:db8::1"), convite.candidatos)
        assertEquals("marcos-desktop", convite.nomeDaMaquina)
    }

    @Test
    fun `a ordem dos campos nao importa`() {
        // O documento diz que a ordem é estável mas que o aplicativo não deve
        // depender dela: depender da posição transforma mudança inofensiva em quebra.
        val embaralhada = "lume://pair?n=marcos&p=43140&c=$codigo&h=&f=$fingerprintEmB64&v=1"
        assertEquals(43140, lerConvite(embaralhada).getOrThrow().porta)
    }

    @Test
    fun `versao desconhecida para tudo`() {
        // Nunca interpretar os demais campos assim mesmo: um formato v2 pode dar
        // outro significado a `f`, e adivinhar aqui é fixar a chave errada.
        assertEquals(FalhaDeLeitura.VersaoDesconhecida(2), motivo(uri(v = "2")))
    }

    @Test
    fun `campo ausente e nomeado`() {
        val semFingerprint = "lume://pair?v=1&c=$codigo&p=43140&h=&n=x"
        assertEquals(FalhaDeLeitura.CampoAusente("f"), motivo(semFingerprint))
    }

    @Test
    fun `fingerprint com comprimento errado e corrompido`() {
        assertEquals(FalhaDeLeitura.CampoCorrompido("f"), motivo(uri(f = "curto")))
    }

    @Test
    fun `fingerprint em base64 padrao e recusado`() {
        // O alfabeto padrão usa `+` e `/`; o base64url usa `-` e `_`. Um QR gerado
        // com o decodificador errado precisa falhar aqui, e não conectar contra
        // bytes que não são o hash.
        val comAlfabetoErrado = Base64.getEncoder().withoutPadding()
            .encodeToString(ByteArray(32) { 0xFB.toByte() })
        assertEquals(FalhaDeLeitura.CampoCorrompido("f"), motivo(uri(f = comAlfabetoErrado)))
    }

    @Test
    fun `lista de candidatos vazia nao e erro`() {
        // `&h=&` acontece quando a máquina não tem endereço não-loopback
        // utilizável. O código e o fingerprint deste mesmo QR ainda pareiam.
        val convite = lerConvite(uri(h = "")).getOrThrow()
        assertTrue(convite.candidatos.isEmpty())
    }

    @Test
    fun `ipv6 chega sem colchetes e ganha colchetes so na url`() {
        val convite = lerConvite(uri(h = "2001:db8::1")).getOrThrow()
        assertEquals("2001:db8::1", convite.candidatos.single())
        // Quem monta a autoridade é o aplicativo. Se o QR trouxesse colchetes, o
        // resultado seria `wss://[[2001:db8::1]]:43140`.
        assertEquals("wss://[2001:db8::1]:43140/lume", convite.enderecos().single().url())
    }

    @Test
    fun `ipv4 nao ganha colchetes`() {
        val convite = lerConvite(uri(h = "192.168.0.14")).getOrThrow()
        assertEquals("wss://192.168.0.14:43140/lume", convite.enderecos().single().url())
    }

    @Test
    fun `a ordem dos candidatos e preservada`() {
        // Significativa: interfaces físicas antes das virtuais. Uma máquina com
        // Docker anuncia 172.17.0.1, que não leva a lugar nenhum vindo de fora.
        val convite = lerConvite(uri(h = "192.168.0.14,172.17.0.1")).getOrThrow()
        assertEquals(listOf("192.168.0.14", "172.17.0.1"), convite.candidatos)
    }

    @Test
    fun `nome da maquina e percent-decoded`() {
        // Nome no Windows aceita espaço e acento, e um `&` literal quebraria a
        // query inteira.
        val convite = lerConvite(uri(n = "PC%20do%20Jo%C3%A3o")).getOrThrow()
        assertEquals("PC do João", convite.nomeDaMaquina)
    }

    @Test
    fun `qr de outra coisa nao e convite`() {
        assertEquals(FalhaDeLeitura.NaoEUmConvite, motivo("https://exemplo.com"))
    }

    @Test
    fun `porta fora do intervalo e corrompida`() {
        assertEquals(FalhaDeLeitura.CampoCorrompido("p"), motivo(uri(p = "70000")))
    }

    @Test
    fun `codigo malformado e corrompido`() {
        assertEquals(FalhaDeLeitura.CampoCorrompido("c"), motivo(uri(c = "abc")))
    }
}
