package lume.ai.ui.theme

import androidx.compose.runtime.Immutable
import androidx.compose.ui.text.ExperimentalTextApi
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontVariation
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import lume.ai.R

/**
 * Tipografia do Lume.
 *
 * ## Por que a Inter vai empacotada
 *
 * O desktop declara a pilha `Inter, ui-sans-serif, system-ui, …` em
 * `src/app.css:2` e conta com a Inter estar instalada no sistema. No Android ela
 * nunca está: sem o arquivo no APK, o fallback é a Roboto, e a identidade some no
 * primeiro quadro. A Inter é OFL, então redistribuir é permitido.
 *
 * ## Por que a variável, e não seis arquivos estáticos
 *
 * O desktop usa pesos que só existem em fonte variável — 650, 700, 720, 750. Um
 * `Font` estático mais próximo (600 ou 700) muda o ritmo de cada linha, e a
 * distância relativa entre os papéis é justamente o que `MOBILE-UI-DESIGN.md`
 * manda preservar. Um `.ttf` variável cobre todos os pesos e ainda pesa menos que
 * o conjunto de estáticos que seria necessário.
 *
 * Isso é o que fixa `minSdk = 26`: abaixo dela `FontVariation` é ignorado e todo
 * texto renderiza em 400 — sem erro de build, sem exceção, só a tipografia
 * achatada. Ver a linha do `minSdk` em `docs/ANDROID.md`.
 *
 * O eixo `opsz` da Inter fica no padrão de propósito: mexer nele muda o desenho
 * das letras por tamanho, e o desktop não mexe.
 *
 * ## Por que papéis próprios, e não os do Material
 *
 * O Material tem `displayLarge`, `titleMedium`, `bodySmall` e mais doze. O design
 * tem seis papéis com nomes próprios. Mapear um no outro obriga a decidir se
 * `label` é `labelSmall` ou `bodySmall`, e a resposta não existe — são
 * vocabulários diferentes. Os seis abaixo carregam os nomes do documento.
 */

/**
 * Um único `.ttf` variável, declarado uma vez por peso em uso.
 *
 * Cada `Font` amarra um `FontWeight` ao valor correspondente do eixo `wght`. Sem
 * essa amarração o Compose escolheria o peso por aproximação e sintetizaria o
 * resto, que é exatamente o que a fonte variável existe para evitar.
 */
// A sobrecarga de `Font` que aceita `variationSettings` ainda é experimental no
// Compose. O `@OptIn` fica aqui, na única função que a usa, em vez de num
// argumento de compilação para o módulo inteiro: assim, no dia em que a API
// mudar, o compilador aponta esta linha e não uma tela qualquer.
@OptIn(ExperimentalTextApi::class)
private fun inter(weight: Int) = Font(
    resId = R.font.inter_variable,
    weight = FontWeight(weight),
    variationSettings = FontVariation.Settings(FontVariation.weight(weight)),
)

val InterFamily = FontFamily(
    inter(400),
    inter(650),
    inter(700),
    inter(720),
    inter(750),
)

/**
 * Monoespaçada do sistema.
 *
 * Não vai fonte empacotada aqui. O desktop também não empacota: a pilha dele é
 * `SFMono-Regular, Consolas, "Liberation Mono", monospace`, ou seja, sempre a
 * fonte do sistema. Trazer uma mono própria daria ao celular mais identidade do
 * que o desktop tem, e paridade é o objetivo.
 */
val MonoFamily = FontFamily.Monospace

@Immutable
data class LumeTypography(
    /** Título de tela e estado vazio. */
    val display: TextStyle,
    /** Cabeçalho de seção, nome do aplicativo. */
    val title: TextStyle,
    /** Nome do agente na linha de sessão, evento no histórico. */
    val titleSmall: TextStyle,
    /** Resposta, prompt, descrição. */
    val body: TextStyle,
    /** Projeto, horário, metadado. */
    val label: TextStyle,
    /** Rótulo de estado e destino da barra inferior, em caixa alta. */
    val overline: TextStyle,
    /** Comando, caminho e saída de ferramenta. */
    val mono: TextStyle,
    /** Texto dentro de botão. */
    val button: TextStyle,
)

/**
 * Os tamanhos são os do desktop reescalados por 1,7–1,8×, como manda
 * `MOBILE-UI-DESIGN.md`. O `letterSpacing` vai em `em`, não em `sp`: em `sp` ele
 * deixaria de acompanhar o corpo quando o usuário aumenta a fonte do sistema, e
 * o rótulo em caixa alta é o primeiro a se desmanchar quando isso acontece.
 */
val LumeTypographyDefault = LumeTypography(
    display = TextStyle(
        fontFamily = InterFamily,
        fontWeight = FontWeight(750),
        fontSize = 24.sp,
        lineHeight = 30.sp,
        letterSpacing = (-0.01).em,
    ),
    title = TextStyle(
        fontFamily = InterFamily,
        fontWeight = FontWeight(720),
        fontSize = 20.sp,
        lineHeight = 26.sp,
    ),
    titleSmall = TextStyle(
        fontFamily = InterFamily,
        fontWeight = FontWeight(700),
        fontSize = 17.sp,
        lineHeight = 23.sp,
    ),
    body = TextStyle(
        fontFamily = InterFamily,
        fontWeight = FontWeight(400),
        fontSize = 15.sp,
        lineHeight = 23.sp,
    ),
    label = TextStyle(
        fontFamily = InterFamily,
        fontWeight = FontWeight(650),
        fontSize = 13.sp,
        lineHeight = 18.sp,
        letterSpacing = 0.025.em,
    ),
    overline = TextStyle(
        fontFamily = InterFamily,
        fontWeight = FontWeight(750),
        fontSize = 11.sp,
        lineHeight = 13.sp,
        letterSpacing = 0.07.em,
    ),
    mono = TextStyle(
        fontFamily = MonoFamily,
        fontWeight = FontWeight(400),
        fontSize = 14.sp,
        lineHeight = 20.sp,
    ),
    // O design pede `line-height:1` aqui. Fica em 1,2 porque a altura do botão é
    // fixa em 48dp: com `fontScale` alto, altura de linha 1 corta as descidas.
    button = TextStyle(
        fontFamily = InterFamily,
        fontWeight = FontWeight(700),
        fontSize = 15.sp,
        lineHeight = 18.sp,
    ),
)
