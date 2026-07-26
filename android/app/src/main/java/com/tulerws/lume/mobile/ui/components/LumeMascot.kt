package com.tulerws.lume.mobile.ui.components

import android.provider.Settings
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.State
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalInspectionMode
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.tulerws.lume.mobile.ui.theme.Brand
import com.tulerws.lume.mobile.ui.theme.LumeTheme

/**
 * O mascote.
 *
 * ## De onde vem a arte
 *
 * Do glifo de `static/lume.svg`, exatamente como o arquivo de design o usa — e
 * **não** do dinossauro de `src/lib/LumeMascot.svelte`. São duas artes diferentes,
 * e o design escolheu a primeira. `docs/MOBILE-UI-DESIGN.md` ainda afirma que os
 * 19 quadros do componente Svelte são portados como estão; a frase ficou
 * desatualizada quando o design foi feito, e está registrada como pendência.
 *
 * ## A gramática: glifo base + glifo de canto na cor do estado
 *
 * Não foi inventada aqui. O design usa a mesma exclamação em `#CB8B45` para
 * permissão pendente e em `#9B4340` na tela de erro, e usa um tique de
 * confirmação para permissão respondida. Dos seis estados ele desenha três; os
 * outros três reusam glifos que ele já definiu, e **só dois** precisaram ser
 * desenhados — [GLIFO_EXECUTANDO] e [GLIFO_ESPERANDO], derivados dos
 * equivalentes de `LumeMascot.svelte` sobre a grade de 6.
 *
 * ## Movimento
 *
 * É movimento de sprite: salta entre quadros, não interpola. Todo deslocamento é
 * `dp` **inteiro**, porque deslocamento fracionário borra a arte e destrói a
 * leitura de pixel — isso é defeito, não detalhe.
 *
 * As cinco fases de transição do desktop (`waking`, `shifting`, `arriving`,
 * `settling`, `falling-asleep`) não estão aqui: elas só aparecem quando o estado
 * muda, e o estado só muda de verdade quando existir `sessions.delta`.
 *
 * O mascote é decorativo e some para o leitor de tela. O que ele diz está sempre
 * escrito ao lado, em texto.
 */
enum class EstadoDoMascote {
    /** Sem conexão. Dorme, com os Z. */
    Dormindo,

    /** Conectado, nenhum agente ativo. */
    Desperto,

    /** Algum agente pedindo permissão. */
    PedindoPermissao,

    /** Algum agente executando. */
    Executando,

    /** Última transição foi conclusão. */
    Concluido,

    /** Última transição foi falha. */
    Falhou,

    /** Algum agente esperando resposta do usuário. */
    Esperando,
}

// A arte vive numa grade de 512 em blocos de 16. O mascote enquadra
// "88 88 336 376" dela — a mesma janela que o design usa.
private const val JANELA_X = 88f
private const val JANELA_Y = 88f
private const val JANELA_LARGURA = 336f
private const val JANELA_ALTURA = 376f
private const val PROPORCAO = JANELA_ALTURA / JANELA_LARGURA

private const val GLIFO_ATENCAO = "M2,0h2v3H2zM2,4h2v2H2z"
private const val GLIFO_CONCLUIDO = "M0,3h2v2H0zM1,4h2v2H1zM2,3h2v2H2zM3,2h2v2H3zM4,1h2v2H4z"
private const val GLIFO_EXECUTANDO = "M1,1h4v1H1zM0,3h5v1H0zM2,5h3v1H2z"
private const val GLIFO_ESPERANDO = "M1,0h3v1H1zM4,1h1v1H4zM3,2h1v1H3zM2,3h1v1H2zM2,5h1v1H2z"

/**
 * Caminhos já convertidos.
 *
 * A conversão do texto SVG para [Path] acontece uma vez, e não a cada quadro. O
 * mascote redesenha várias vezes por segundo; reanalisar nove caminhos a cada
 * passada seria trabalho puro em cima do laço mais quente da tela.
 *
 * Os dados de caminho são os literais do arquivo de design, sem redesenho: é o
 * que garante que a arte na tela é a arte aprovada, e não uma transcrição.
 */
private object Arte {
    private fun p(dados: String): Path = PathParser().parsePathString(dados).toPath()

    val sombra = p("M96,128h32V96h256v32h32v32h32v160H256v32h128v64h-32v32H192v-32h-64v-32H96z")
    val corpo = p("M128,160h32v-32h192v32h32v32h32v96H224v32h-32v64h-32v-32h-32z")
    val realce = p("M160,160h160v32h-160z")
    val olhoBranco = p("M304,176h64v64h-64z")
    val pupila = p("M336,192h32v32h-32z")
    val olhoFechado = p("M304,200h64v16h-64z")
    val detalhePata = p("M384,256h32v16h-32z")
    val ventre = p("M224,288h192v32H256v32h-32z")
    val dente = p("M320,320h32v24h-32z")

    val zGrande = p("M8,4h5v2H8zM11,6h2v2h-2zM9,8h2v2H9zM8,10h5v2H8z")
    val zPequeno = p("M2,0h4v2H2zM4,2h2v1H4zM2,3h2v1H2zM2,4h4v2H2z")

    private val glifos = mutableMapOf<String, Path>()
    fun glifo(dados: String): Path = glifos.getOrPut(dados) { p(dados) }
}

@Composable
fun LumeMascot(
    estado: EstadoDoMascote,
    modifier: Modifier = Modifier,
    largura: Dp = 96.dp,
) {
    val cores = LumeTheme.states

    val glifoDeCanto: Pair<String, Color>? = when (estado) {
        EstadoDoMascote.PedindoPermissao -> GLIFO_ATENCAO to cores.permissionRequired.fill
        EstadoDoMascote.Falhou -> GLIFO_ATENCAO to cores.failed.fill
        EstadoDoMascote.Concluido -> GLIFO_CONCLUIDO to cores.completed.fill
        EstadoDoMascote.Executando -> GLIFO_EXECUTANDO to cores.running.fill
        EstadoDoMascote.Esperando -> GLIFO_ESPERANDO to cores.waitingForInput.fill
        EstadoDoMascote.Dormindo, EstadoDoMascote.Desperto -> null
    }
    val dormindo = estado == EstadoDoMascote.Dormindo
    val corDosZ = cores.idle.on

    val animar = movimentoPermitido()

    // Duas frequências, como no design: 2,4s quando não há nada acontecendo e
    // 1,9s quando há. É a diferença entre ocioso e atento.
    val duracaoDoBalanco = when (estado) {
        EstadoDoMascote.Dormindo -> 2_600
        EstadoDoMascote.Desperto -> 2_400
        else -> 1_900
    }

    val balanco = contador(duracaoDoBalanco, ate = 2f, ativo = animar)
    val piscada = contador(4_300, ate = 100f, ativo = animar)
    val bandeira = contador(620, ate = 2f, ativo = animar)
    val sono = contador(2_600, ate = 4f, ativo = animar)

    Canvas(
        modifier = modifier
            .size(width = largura, height = largura * PROPORCAO)
            .clearAndSetSemantics { },
    ) {
        val escala = size.width / JANELA_LARGURA

        // `lm-bob2` sobe 2dp; `lm-sleep` desce 1dp. Sempre inteiro.
        val deslocamentoY = when {
            !animar -> 0f
            dormindo -> if (balanco.value >= 1f) 1.dp.toPx() else 0f
            else -> if (balanco.value >= 1f) -2.dp.toPx() else 0f
        }

        withTransform({
            translate(0f, deslocamentoY)
            scale(escala, escala, Offset.Zero)
            translate(-JANELA_X, -JANELA_Y)
        }) {
            drawPath(Arte.sombra, Brand.Shadow)
            drawPath(Arte.corpo, Brand.Body)
            drawPath(Arte.realce, Brand.Highlight)
            drawPath(Arte.olhoBranco, Brand.Light)
            if (dormindo) {
                drawPath(Arte.olhoFechado, Brand.DetailDark)
            } else {
                // A piscada ocupa os últimos 4% do ciclo — 91% a 95% no original.
                val piscando = animar && piscada.value >= 91f && piscada.value < 95f
                if (!piscando) drawPath(Arte.pupila, Brand.DetailDark)
            }
            drawPath(Arte.detalhePata, Brand.DetailMid)
            drawPath(Arte.ventre, Brand.Background)
            drawPath(Arte.dente, Brand.Light)
        }

        if (glifoDeCanto != null) {
            val (dados, cor) = glifoDeCanto
            val lado = 24.dp.toPx()
            // Canto superior direito, transbordando 8dp para fora da arte, como
            // no design. O balanço do glifo é o dobro da frequência do corpo.
            val subida = if (animar && bandeira.value >= 1f) -3.dp.toPx() else 0f
            // Convertidos aqui: dentro de `withTransform` o receptor é
            // `DrawTransform`, que não é `Density`, e `Dp.toPx()` some.
            val transbordo = 8.dp.toPx()
            val topo = 2.dp.toPx()
            withTransform({
                translate(size.width - lado + transbordo, topo + subida)
                scale(lado / 6f, lado / 6f, Offset.Zero)
            }) {
                drawPath(Arte.glifo(dados), cor)
            }
        }

        if (dormindo) {
            val lado = 34.dp.toPx()
            val quadroDoSono = if (animar) sono.value.toInt().coerceIn(0, 3) else 1
            val folgaDosZ = 14.dp.toPx()
            withTransform({
                translate(size.width - lado + folgaDosZ, -folgaDosZ + deslocamentoY)
            }) {
                desenharZ(Arte.zGrande, corDosZ, quadroDoSono, lado)
                // O segundo Z entra quase um ciclo depois, como o `animation-delay`
                // de 0,9s do design.
                desenharZ(Arte.zPequeno, corDosZ.copy(alpha = 0.7f), (quadroDoSono + 3) % 4, lado)
            }
        }
    }
}

/**
 * Um Z do sono.
 *
 * `lm-z` percorre opacidade 0 → 1 → 1 → 0 enquanto sobe e vai para a direita. Em
 * quatro passos: 0 e 3 invisíveis, 1 e 2 visíveis subindo. Devolver cedo no
 * quadro invisível evita desenhar caminho com alfa zero.
 */
private fun DrawScope.desenharZ(path: Path, cor: Color, quadro: Int, lado: Float) {
    if (quadro != 1 && quadro != 2) return
    val passo = 2.dp.toPx() * quadro
    withTransform({
        translate(passo, -2f * passo)
        scale(lado / 17f, lado / 17f, Offset.Zero)
    }) {
        drawPath(path, cor)
    }
}

/**
 * Contador cíclico de 0 a [ate].
 *
 * O valor é contínuo só para o relógio andar: quem desenha compara com `>=` ou
 * trunca com `toInt()`, e nenhum valor fracionário chega à tela.
 */
@Composable
private fun contador(duracaoMs: Int, ate: Float, ativo: Boolean): State<Float> {
    if (!ativo) return remember { mutableFloatStateOf(0f) }
    val transicao = rememberInfiniteTransition(label = "mascote")
    return transicao.animateFloat(
        initialValue = 0f,
        targetValue = ate,
        animationSpec = infiniteRepeatable(
            animation = tween(duracaoMs, easing = LinearEasing),
            repeatMode = RepeatMode.Restart,
        ),
        label = "quadro",
    )
}

/**
 * Se o mascote pode animar.
 *
 * Respeita `ANIMATOR_DURATION_SCALE`: em zero ele congela no quadro de repouso, e
 * o estado segue legível pela cor do glifo e pelo texto ao lado — a animação nunca
 * é a única portadora de informação.
 *
 * No `@Preview` também congela: quadro parado é o que se compara com o design.
 */
@Composable
private fun movimentoPermitido(): Boolean {
    if (LocalInspectionMode.current) return false
    val contexto = LocalContext.current
    return remember(contexto) {
        Settings.Global.getFloat(
            contexto.contentResolver,
            Settings.Global.ANIMATOR_DURATION_SCALE,
            1f,
        ) != 0f
    }
}
