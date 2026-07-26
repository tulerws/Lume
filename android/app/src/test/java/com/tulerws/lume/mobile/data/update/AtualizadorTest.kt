package com.tulerws.lume.mobile.data.update

import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * As duas peças do atualizador que decidem sozinhas, sem Android por perto.
 *
 * O download e a instalação não estão aqui de propósito: eles dependem de
 * `PackageManager` e do sistema de arquivos, e um teste que os simulasse estaria
 * testando o simulacro. O que **pode** falhar em silêncio e virar defeito real é
 * a aritmética de versão e o portão de origem — e é isso que este arquivo cobre.
 *
 * Rodam na JVM porque nenhuma das duas usa `android.*`. Foi por isso que elas não
 * usam.
 */
class AtualizadorTest {

    // ── comparação de versões ─────────────────────────────────────────────────

    /**
     * Os números que **já estão publicados**. Se este teste mudar de valor, a
     * atualização deixa de instalar em quem tem a v0.5.3 — o teste existe para que
     * isso não passe despercebido.
     */
    @Test
    fun `reproduz os codigos ja publicados`() {
        assertEquals(5_000_000, codigoDaVersao("0.5.0"))
        assertEquals(5_003_000, codigoDaVersao("0.5.3"))
        assertEquals(5_004_000, codigoDaVersao("0.5.4"))
    }

    /** A armadilha clássica: como texto, `"0.5.10" < "0.5.9"`. */
    @Test
    fun `patch de dois digitos supera o de um`() {
        assertTrue(codigoDaVersao("0.5.10") > codigoDaVersao("0.5.9"))
    }

    @Test
    fun `minor supera qualquer patch anterior`() {
        assertTrue(codigoDaVersao("0.6.0") > codigoDaVersao("0.5.999"))
    }

    @Test
    fun `major supera qualquer minor anterior`() {
        assertTrue(codigoDaVersao("1.0.0") > codigoDaVersao("0.99.999"))
        assertEquals(100_000_000, codigoDaVersao("1.0.0"))
    }

    /** Candidata e final compartilham o código; ver o KDoc de [codigoDaVersao]. */
    @Test
    fun `sufixo de pre-lancamento e descartado`() {
        assertEquals(codigoDaVersao("0.6.0"), codigoDaVersao("0.6.0-rc1"))
    }

    @Test
    fun `versao curta e completada com zeros`() {
        assertEquals(codigoDaVersao("0.6.0"), codigoDaVersao("0.6"))
    }

    /**
     * Entrada que não dá para representar devolve `0`, e `0` nunca é maior que a
     * versão instalada — o aplicativo se cala em vez de anunciar o que não pode
     * instalar. As duas primeiras colidiriam com `1.0.0` e `0.6.0` se passassem.
     */
    @Test
    fun `fora de faixa e lixo devolvem zero`() {
        assertEquals(0, codigoDaVersao("0.100.0"))
        assertEquals(0, codigoDaVersao("0.5.1000"))
        assertEquals(0, codigoDaVersao(""))
        assertEquals(0, codigoDaVersao("nao-e-versao"))
        assertEquals(0, codigoDaVersao("1.2.3.4.5"))
    }

    // ── portão de origem ──────────────────────────────────────────────────────

    /** O endereço exatamente como o gerador de manifesto o escreve. */
    @Test
    fun `aceita o endereco publicado de verdade`() {
        assertTrue(
            urlConfiavel(
                "https://github.com/tulerws/Lume/releases/download/v0.5.3/Lume-Mobile.apk",
            ),
        )
    }

    /**
     * Os negativos são a razão de este portão existir. Cada linha é uma forma
     * conhecida de um manifesto adulterado tentar apontar o download para fora.
     */
    @Test
    fun `recusa tudo que nao seja o repositorio em https`() {
        // Texto puro: seria interceptável, e o `usesCleartextTraffic=false` já o
        // barraria — mas depois de o endereço ter sido aceito como legítimo.
        assertFalse(urlConfiavel("http://github.com/tulerws/Lume/releases/download/v1/a.apk"))
        // Domínio que *contém* o esperado. O erro de quem compara com `contains`.
        assertFalse(urlConfiavel("https://github.com.exemplo.net/tulerws/Lume/releases/download/v1/a.apk"))
        // Subdomínio: o certificado seria válido, o host não é o nosso.
        assertFalse(urlConfiavel("https://raw.github.com/tulerws/Lume/releases/download/v1/a.apk"))
        // Outro repositório no mesmo host.
        assertFalse(urlConfiavel("https://github.com/outro/Lume/releases/download/v1/a.apk"))
        // Mesmo repositório, caminho que não é o de download de release.
        assertFalse(urlConfiavel("https://github.com/tulerws/Lume/raw/main/a.apk"))
        // `github.com` no campo de usuário, host de verdade em outro domínio: a
        // URL *parece* apontar para o GitHub para quem a lê da esquerda para a
        // direita. Recusada porque `host` devolve `exemplo.net`.
        assertFalse(urlConfiavel("https://github.com@exemplo.net/tulerws/Lume/releases/download/v1/a.apk"))
        assertFalse(urlConfiavel(""))
        assertFalse(urlConfiavel("nao e uma url"))
    }

    /**
     * Documentado como comportamento conhecido, e não como descuido:
     * `docs/ANDROID.md`, *O que o portão de origem não recusa*.
     *
     * Credencial embutida com o host verdadeiro **passa** — `host` continua sendo
     * `github.com`, que é o que o portão pergunta. Não é abertura: o destino segue
     * fixado no caminho e o conteúdo segue conferido por SHA-256. Este teste existe
     * para que a diferença em relação ao caso acima não se perca.
     */
    @Test
    fun `credencial embutida com host verdadeiro passa`() {
        assertTrue(
            urlConfiavel(
                "https://usuario:senha@github.com/tulerws/Lume/releases/download/v1/a.apk",
            ),
        )
    }

    /** O host não diferencia maiúsculas; o caminho, sim. */
    @Test
    fun `host e insensivel a caixa e caminho nao e`() {
        assertTrue(urlConfiavel("https://GitHub.COM/tulerws/Lume/releases/download/v1/a.apk"))
        assertFalse(urlConfiavel("https://github.com/TULERWS/Lume/releases/download/v1/a.apk"))
    }

    // ── leitura do manifesto ──────────────────────────────────────────────────

    private val json = Json { ignoreUnknownKeys = true }

    /** Byte a byte o que está publicado hoje em `mobile-latest.json`. */
    private val publicado = """
        {
          "version": "0.5.3",
          "publishedAt": "2026-07-26T05:07:24.027Z",
          "notesUrl": "https://github.com/tulerws/Lume/releases/tag/v0.5.3",
          "android": {
            "url": "https://github.com/tulerws/Lume/releases/download/v0.5.3/Lume-Mobile.apk",
            "sha256": "84d81461f81f9e4c35630c4f8ea23566758ebe5c43495a96620a9ebd92cb63f2"
          }
        }
    """.trimIndent()

    @Test
    fun `le o manifesto publicado`() {
        val m = json.decodeFromString(ManifestoDeAtualizacao.serializer(), publicado)
        assertEquals("0.5.3", m.version)
        assertEquals(
            "https://github.com/tulerws/Lume/releases/download/v0.5.3/Lume-Mobile.apk",
            m.android.url,
        )
        assertEquals(64, m.android.sha256.length)
        assertTrue(urlConfiavel(m.android.url))
    }

    /**
     * Campo novo no gerador não pode derrubar a leitura em aparelhos já instalados
     * — eles não têm como receber um modelo atualizado antes de atualizar.
     */
    @Test
    fun `campo desconhecido nao derruba a leitura`() {
        val comExtra = publicado.replace(
            "\"version\": \"0.5.3\",",
            "\"version\": \"0.5.3\",\n  \"canalDeDistribuicao\": \"estavel\",",
        )
        val m = json.decodeFromString(ManifestoDeAtualizacao.serializer(), comExtra)
        assertEquals("0.5.3", m.version)
    }

    /** Os acessórios são opcionais; `version` e `android` não são. */
    @Test
    fun `manifesto minimo e aceito`() {
        val minimo = """
            {"version":"0.9.0","android":{"url":"https://github.com/tulerws/Lume/releases/download/v0.9.0/Lume-Mobile.apk","sha256":"${"a".repeat(64)}"}}
        """.trimIndent()
        val m = json.decodeFromString(ManifestoDeAtualizacao.serializer(), minimo)
        assertNull(m.notesUrl)
        assertNull(m.publishedAt)
        assertEquals("0.9.0", m.version)
    }
}
