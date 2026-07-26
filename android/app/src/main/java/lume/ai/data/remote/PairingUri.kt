package lume.ai.data.remote

import java.net.URLDecoder
import java.util.Base64

/**
 * O convite que o QR carrega.
 *
 * `lume://pair?v=1&f=<fingerprint>&c=<codigo>&p=43140&h=<host1,host2>&n=<nome>`
 *
 * [candidatos] pode vir **vazio**, e isso não é QR inválido: significa que a
 * máquina não tem endereço não-loopback utilizável, e que o caminho é mDNS ou
 * endereço digitado. O código e o fingerprint deste mesmo QR completam o
 * pareamento de qualquer jeito.
 */
data class ConviteDePareamento(
    val fingerprint: ByteArray,
    val codigo: String,
    val porta: Int,
    val candidatos: List<String>,
    val nomeDaMaquina: String,
) {
    fun enderecos(): List<Endereco> = candidatos.map { Endereco(it, porta) }

    override fun equals(other: Any?): Boolean =
        this === other ||
            (other is ConviteDePareamento && fingerprint.contentEquals(other.fingerprint) &&
                codigo == other.codigo && porta == other.porta &&
                candidatos == other.candidatos && nomeDaMaquina == other.nomeDaMaquina)

    override fun hashCode(): Int =
        listOf(fingerprint.contentHashCode(), codigo, porta, candidatos, nomeDaMaquina).hashCode()
}

/**
 * Por que a leitura falhou.
 *
 * Cada caso vira uma frase diferente na tela. "QR inválido" para tudo deixaria o
 * usuário sem saber se deve reaproximar a câmera, atualizar o computador ou
 * reabrir a janela de pareamento.
 */
sealed interface FalhaDeLeitura {
    /** Não é um `lume://pair`. Provavelmente o usuário mirou noutro QR. */
    data object NaoEUmConvite : FalhaDeLeitura

    /**
     * Versão que este aplicativo não conhece.
     *
     * O documento é explícito: **dizer ao usuário e parar**. Nunca interpretar os
     * demais campos assim mesmo — um formato v2 pode dar outro significado a
     * `f` ou a `c`, e adivinhar aqui é fixar a chave errada.
     */
    data class VersaoDesconhecida(val versao: Int) : FalhaDeLeitura

    data class CampoAusente(val campo: String) : FalhaDeLeitura

    /** Comprimento ou alfabeto errado. 43 caracteres base64url, 32 bytes. */
    data class CampoCorrompido(val campo: String) : FalhaDeLeitura
}

/** Versão de formato que este aplicativo sabe ler. */
private const val VERSAO_SUPORTADA = 1

private const val ESQUEMA = "lume://pair"

/**
 * Lê o texto de um QR.
 *
 * ## Por que analisado à mão, e não com `android.net.Uri`
 *
 * `Uri` é classe de plataforma: um teste de unidade que a use precisa de
 * Robolectric ou de aparelho. Este parser é o que o documento manda testar com
 * URI válida, versão desconhecida, campo ausente, fingerprint malformado e lista
 * de candidatos vazia — cinco casos que devem rodar na JVM, em milissegundos, no
 * mesmo `gradlew test` de todo dia.
 *
 * ## Detalhes que decidem se a leitura funciona
 *
 * · **A ordem dos campos não importa.** Ela é estável na geração, mas depender da
 *   posição transformaria uma mudança inofensiva em quebra.
 * · **`f` e `c` são base64url sem preenchimento.** Um decodificador de base64
 *   padrão falha no `-` e no `_`; um que exija preenchimento falha no
 *   comprimento 43.
 * · **IPv6 vem sem colchetes** e assim permanece aqui. Quem monta a autoridade da
 *   URL é [Endereco.url], e é ele que acrescenta — senão sairia `[[2001:db8::1]]`.
 */
fun lerConvite(texto: String): Result<ConviteDePareamento> {
    val cru = texto.trim()
    if (!cru.startsWith(ESQUEMA)) return falha(FalhaDeLeitura.NaoEUmConvite)

    val query = cru.substringAfter('?', missingDelimiterValue = "")
    val campos = query.split('&')
        .filter { it.isNotEmpty() }
        .mapNotNull { par ->
            val chave = par.substringBefore('=')
            if (chave.isEmpty()) null else chave to par.substringAfter('=', "")
        }
        .toMap()

    // A versão é conferida **antes** de qualquer outro campo ser interpretado.
    val versao = campos["v"]?.toIntOrNull() ?: return falha(FalhaDeLeitura.CampoAusente("v"))
    if (versao != VERSAO_SUPORTADA) return falha(FalhaDeLeitura.VersaoDesconhecida(versao))

    val fingerprintTexto = campos["f"] ?: return falha(FalhaDeLeitura.CampoAusente("f"))
    val fingerprint = decodificar32Bytes(fingerprintTexto)
        ?: return falha(FalhaDeLeitura.CampoCorrompido("f"))

    val codigo = campos["c"] ?: return falha(FalhaDeLeitura.CampoAusente("c"))
    // O código é repassado como texto, não decodificado: é assim que ele viaja no
    // cabeçalho `X-Lume-Pairing-Code`, e decodificar para recodificar só criaria
    // uma chance de o valor mudar no caminho. A validação confere a forma.
    if (decodificar32Bytes(codigo) == null) return falha(FalhaDeLeitura.CampoCorrompido("c"))

    val porta = campos["p"]?.toIntOrNull() ?: return falha(FalhaDeLeitura.CampoAusente("p"))
    if (porta !in 1..65535) return falha(FalhaDeLeitura.CampoCorrompido("p"))

    // `h` ausente e `h` vazio são a mesma coisa: nenhum candidato. Nenhum dos dois
    // é erro — o pareamento segue por mDNS ou por endereço digitado.
    val candidatos = campos["h"].orEmpty()
        .split(',')
        .map { it.trim() }
        .filter { it.isNotEmpty() }

    val nome = campos["n"].orEmpty().let { valor ->
        runCatching { URLDecoder.decode(valor, "UTF-8") }.getOrDefault(valor)
    }

    return Result.success(
        ConviteDePareamento(
            fingerprint = fingerprint,
            codigo = codigo,
            porta = porta,
            candidatos = candidatos,
            nomeDaMaquina = nome,
        ),
    )
}

/**
 * base64url sem preenchimento, para exatamente 32 bytes.
 *
 * Comprimento diferente de 32 é QR corrompido, e recusar aqui é melhor do que
 * conectar comparando contra bytes que não são um SHA-256.
 */
private fun decodificar32Bytes(texto: String): ByteArray? {
    if (texto.length != 43) return null
    val bytes = runCatching { Base64.getUrlDecoder().decode(texto) }.getOrNull() ?: return null
    return bytes.takeIf { it.size == 32 }
}

private fun falha(motivo: FalhaDeLeitura): Result<ConviteDePareamento> =
    Result.failure(LeituraDeConviteFalhou(motivo))

class LeituraDeConviteFalhou(val motivo: FalhaDeLeitura) : Exception(motivo.toString())
