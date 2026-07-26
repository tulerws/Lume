package com.tulerws.lume.mobile.data.update

import kotlinx.serialization.Serializable

/**
 * O `mobile-latest.json` publicado a cada release.
 *
 * Os nomes dos campos **não são escolha nossa**: eles espelham o que
 * `scripts/mobile-release-manifest.mjs` já gera e o que está publicado desde a
 * v0.5.0. Renomear qualquer um aqui não renomeia lá — só quebra a leitura.
 *
 * ```json
 * {
 *   "version": "0.5.3",
 *   "publishedAt": "2026-07-26T05:07:24.027Z",
 *   "notesUrl": "https://github.com/tulerws/Lume/releases/tag/v0.5.3",
 *   "android": { "url": "https://…/Lume-Mobile.apk", "sha256": "84d81…" }
 * }
 * ```
 *
 * `publishedAt` e `notesUrl` são opcionais no modelo ainda que o gerador sempre os
 * escreva: um manifesto antigo, ou um gerador que mude, não deve derrubar a
 * checagem inteira por um campo acessório. `version` e `android` são obrigatórios
 * porque sem eles não há o que decidir nem o que baixar.
 */
@Serializable
data class ManifestoDeAtualizacao(
    val version: String,
    val android: ArtefatoAndroid,
    val publishedAt: String? = null,
    val notesUrl: String? = null,
)

@Serializable
data class ArtefatoAndroid(
    val url: String,
    /** SHA-256 do APK em hexadecimal minúsculo, 64 caracteres. */
    val sha256: String,
)

/**
 * O mesmo número que o Android usa para decidir se aceita a instalação.
 *
 * Esta é a razão de a comparação de versões não ser textual nem "semântica": o
 * sistema não compara `"0.5.10"` com `"0.5.9"`, ele compara dois inteiros. Se
 * usássemos outro critério aqui, existiria um caso em que a tela anuncia
 * atualização e o instalador a recusa como retrocesso — a pior combinação
 * possível, porque a pessoa vê o aviso e não consegue agir sobre ele.
 *
 * Usando exatamente a fórmula de `app/build.gradle.kts`, a resposta desta função e
 * a decisão do sistema **não podem divergir**.
 *
 * Entrada inesperada devolve `0`, que nunca é maior que a versão instalada — na
 * dúvida, o aplicativo se cala em vez de anunciar uma atualização que não existe.
 */
fun codigoDaVersao(nome: String): Int {
    val partes = nome.trim().split(".")
        .map { parte -> Regex("^\\d+").find(parte)?.value?.toIntOrNull() ?: 0 }
        .let { it + List(maxOf(0, 4 - it.size)) { 0 } }

    // Uma versão com mais de quatro componentes não é algo que este projeto gere;
    // truncar silenciosamente seria adivinhar. `0` cai no caso "não anuncia nada".
    if (partes.size > 4) return 0
    if (partes[1] > 99 || partes[2] > 999 || partes[3] > 999) return 0

    return (partes[0] * 100_000_000) +
        (partes[1] * 1_000_000) +
        (partes[2] * 1_000) +
        partes[3]
}
