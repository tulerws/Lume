package lume.ai.data.remote

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate

/**
 * Rust e Android concordam sobre os mesmos bytes.
 *
 * Este é o teste que `docs/REMOTE-CONTROL.md` aponta como o que fecha um dos
 * riscos em aberto: *"se o certificado que o `rustls` apresenta produz, em
 * `cert.encoded` do lado Kotlin, exatamente os bytes que o desktop hasheou. Um
 * teste com fixture gerada pelo lado Rust resolve isso sem aparelho."*
 *
 * A fixture em `src/test/resources/` **não foi escrita à mão**. Ela saiu de
 * `RemoteIdentity::load_or_create`, o mesmo caminho que roda em produção, com o
 * mesmo `rcgen` e as mesmas features do `Cargo.toml`. O `identity.fingerprint.txt`
 * é o que `fingerprint_of` calculou do lado Rust, conferido de forma independente
 * por `sha256sum`.
 *
 * O que ele prova, e nenhum outro teste deste módulo prova: que o
 * `CertificateFactory` do Android reserializa o DER do `rcgen` **sem alterar um
 * byte**. Se `X509Certificate.getEncoded()` normalizasse qualquer coisa — ordem
 * de campo, codificação de inteiro, o que for —, o fingerprint mudaria, o
 * pinning nunca casaria, e a falha chegaria como "não conecta" sem pista nenhuma.
 *
 * Para regerar depois de o formato do certificado mudar do lado Rust, veja o
 * procedimento no fim deste arquivo.
 */
class FixtureDoRustTest {

    private val derDoRust: ByteArray =
        checkNotNull(javaClass.getResourceAsStream("/identity.der")) {
            "fixture ausente: src/test/resources/identity.der"
        }.use { it.readBytes() }

    private val fingerprintDoRust: String =
        checkNotNull(javaClass.getResourceAsStream("/identity.fingerprint.txt")) {
            "fixture ausente: src/test/resources/identity.fingerprint.txt"
        }.use { it.readBytes().decodeToString().trim() }

    private val certificado: X509Certificate =
        CertificateFactory.getInstance("X.509")
            .generateCertificate(derDoRust.inputStream()) as X509Certificate

    @Test
    fun `o Android nao altera o DER que o rcgen produziu`() {
        // A ida e volta pelo `CertificateFactory` precisa ser idêntica byte a
        // byte. É disto que o fingerprint depende.
        assertArrayEquals(derDoRust, certificado.encoded)
    }

    @Test
    fun `o fingerprint calculado aqui e o mesmo que o desktop calculou`() {
        val calculado = fingerprintDe(certificado).joinToString("") { "%02x".format(it) }
        assertEquals(fingerprintDoRust, calculado)
    }

    @Test
    fun `o gestor aceita o certificado real do desktop`() {
        val fixado = fingerprintDoRust.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
        assertEquals(32, fixado.size)
        // Sem exceção: é o caminho feliz completo, do `rcgen` ao portão.
        TrustManagerFixado(fixado).checkServerTrusted(arrayOf(certificado), "ECDHE_ECDSA")
    }

    @Test
    fun `o certificado do desktop e autoassinado`() {
        // Confirma a premissa do desenho inteiro: não há autoridade acima dele,
        // e por isso nenhum mecanismo baseado em cadeia poderia validá-lo.
        assertEquals(certificado.subjectX500Principal, certificado.issuerX500Principal)
    }

    @Test
    fun `o SAN existe mas nao decide nada`() {
        // `REMOTE-CONTROL.md`: o SAN existe para mensagem de erro legível e
        // diagnóstico. Este teste registra que ele está lá — e o teste do
        // verificador de hostname registra que ele não é consultado.
        val nomes = certificado.subjectAlternativeNames
        assertTrue("o certificado deveria trazer SAN para diagnóstico", !nomes.isNullOrEmpty())
    }
}

/*
 * ─── Como regerar a fixture ──────────────────────────────────────────────────
 *
 * Necessário apenas se o formato do certificado mudar em `remote_identity.rs`.
 * Acrescente ao fim daquele arquivo, rode, e desfaça com `git checkout`:
 *
 *     #[cfg(test)]
 *     mod fixture_para_android {
 *         use super::*;
 *         #[test]
 *         #[ignore]
 *         fn emite_fixture() {
 *             let dir = std::env::temp_dir().join("lume-fixture-android");
 *             let _ = std::fs::remove_dir_all(&dir);
 *             let identity = RemoteIdentity::load_or_create(&dir).expect("identidade");
 *             let destino = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
 *                 .parent().unwrap().join("android/app/src/test/resources");
 *             std::fs::write(destino.join("identity.der"), identity.certificate()).unwrap();
 *             std::fs::write(
 *                 destino.join("identity.fingerprint.txt"),
 *                 identity.fingerprint().iter().map(|b| format!("{b:02x}")).collect::<String>(),
 *             ).unwrap();
 *         }
 *     }
 *
 *     cargo test --manifest-path src-tauri/Cargo.toml --lib fixture_para_android -- --ignored
 *
 * O gerador fica fora do repositório de propósito: ele não precisa rodar no CI,
 * e um teste que escreve dentro de `android/` a cada `cargo test` seria um
 * acoplamento entre as duas árvores que ninguém pediu.
 */
