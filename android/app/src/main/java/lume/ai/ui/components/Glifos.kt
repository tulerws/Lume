package lume.ai.ui.components

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import lume.ai.ui.theme.LumeTheme

/**
 * Ícones.
 *
 * A fronteira entre os dois tipos é a regra de `docs/MOBILE-UI-DESIGN.md`, e não
 * deve ser borrada: **pixel para o que é do Lume, Material para o que é do
 * Android.**
 *
 * · Destinos da barra inferior e glifos de estado nascem de grade de pixel, sem
 *   antialiasing de traço, porque ficam ao lado de um logo pixel-art construído em
 *   blocos. Ícone de traço arredondado do Material ali destoa na hora.
 *
 * · Voltar, fechar e enviar são traço do Material. Desenhá-los em pixel-art
 *   prejudicaria o reconhecimento sem ganhar identidade nenhuma — ninguém precisa
 *   de uma seta de voltar autoral.
 *
 * Os caminhos são os do arquivo de design, transcritos sem redesenho.
 */

private object Caminhos {
    private val cache = mutableMapOf<String, Path>()
    fun de(dados: String): Path = cache.getOrPut(dados) { PathParser().parsePathString(dados).toPath() }
}

// Grade de 11, do design. As três compartilham a moldura e mudam só o miolo, o
// que é o que faz elas parecerem uma família e não três desenhos.
private const val ICONE_SESSOES =
    "M1,1h9v1H1zM1,9h9v1H1zM1,2h1v7H1zM9,2h1v7H9zM4,4h3v3H4z"
private const val ICONE_HISTORICO =
    "M1,1h9v1H1zM1,9h9v1H1zM1,2h1v7H1zM9,2h1v7H9zM5,2h1v7H5z"
private const val ICONE_AJUSTES =
    "M4,0h3v2H4zM4,9h3v2H4zM0,4h2v3H0zM9,4h2v3H9zM2,2h2v2H2zM7,2h2v2H7zM2,7h2v2H2zM7,7h2v2H7zM4,4h3v3H4z"

// Grade de 6, a mesma dos glifos de canto do mascote. São literalmente os mesmos
// dois glifos: o tique de "concluído" e a exclamação de "atenção". Reusar em vez
// de desenhar variantes é o que faz a tela de erro parecer parte do mesmo produto
// que o mascote.
private const val GLIFO_TIQUE = "M0,3h2v2H0zM1,4h2v2H1zM2,3h2v2H2zM3,2h2v2H3zM4,1h2v2H4z"
private const val GLIFO_EXCLAMACAO = "M2,0h2v3H2zM2,4h2v2H2z"

// Grade de 24, traço do Material.
private const val TRACO_VOLTAR = "M19,12H5 M12,19l-7,-7 7,-7"
private const val TRACO_FECHAR = "M6,6l12,12 M18,6L6,18"
private const val TRACO_SETA = "M4,12h16 M13,5l7,7 -7,7"

@Composable
private fun GlifoPreenchido(dados: String, cor: Color, grade: Float, tamanho: Dp, modifier: Modifier) {
    Canvas(modifier.size(tamanho).clearAndSetSemantics { }) {
        withTransform({ scale(size.width / grade, size.height / grade, Offset.Zero) }) {
            drawPath(Caminhos.de(dados), cor)
        }
    }
}

@Composable
private fun GlifoTracado(dados: String, cor: Color, tamanho: Dp, modifier: Modifier) {
    Canvas(modifier.size(tamanho).clearAndSetSemantics { }) {
        val escala = size.width / 24f
        withTransform({ scale(escala, escala, Offset.Zero) }) {
            drawPath(
                path = Caminhos.de(dados),
                color = cor,
                // Espessura 2 na grade de 24, ponta e junta arredondadas: é o
                // peso 400 do Material Symbols, e é o que o design desenha.
                style = Stroke(width = 2f, cap = StrokeCap.Round, join = StrokeJoin.Round),
            )
        }
    }
}

@Composable
fun IconeSessoes(cor: Color, modifier: Modifier = Modifier, tamanho: Dp = 22.dp) =
    GlifoPreenchido(ICONE_SESSOES, cor, 11f, tamanho, modifier)

@Composable
fun IconeHistorico(cor: Color, modifier: Modifier = Modifier, tamanho: Dp = 22.dp) =
    GlifoPreenchido(ICONE_HISTORICO, cor, 11f, tamanho, modifier)

@Composable
fun IconeAjustes(cor: Color, modifier: Modifier = Modifier, tamanho: Dp = 22.dp) =
    GlifoPreenchido(ICONE_AJUSTES, cor, 11f, tamanho, modifier)

/** Tique de confirmação, o mesmo glifo que o mascote usa para "concluído". */
@Composable
fun GlifoDeConfirmacao(modifier: Modifier = Modifier, tamanho: Dp = 20.dp) =
    GlifoPreenchido(GLIFO_TIQUE, LumeTheme.states.completed.fill, 6f, tamanho, modifier)

/**
 * Exclamação de atenção.
 *
 * Cor de falha por padrão, que é como a tela de erro a usa. No mascote a mesma
 * forma aparece em `permissionRequired` — a cor é que diz qual dos dois.
 */
@Composable
fun GlifoDeAtencao(
    modifier: Modifier = Modifier,
    tamanho: Dp = 36.dp,
    cor: Color = LumeTheme.states.failed.on,
) = GlifoPreenchido(GLIFO_EXCLAMACAO, cor, 6f, tamanho, modifier)

@Composable
fun IconeVoltar(cor: Color, modifier: Modifier = Modifier, tamanho: Dp = 24.dp) =
    GlifoTracado(TRACO_VOLTAR, cor, tamanho, modifier)

@Composable
fun IconeFechar(cor: Color, modifier: Modifier = Modifier, tamanho: Dp = 24.dp) =
    GlifoTracado(TRACO_FECHAR, cor, tamanho, modifier)

@Composable
fun IconeEnviar(cor: Color, modifier: Modifier = Modifier, tamanho: Dp = 22.dp) =
    GlifoTracado(TRACO_SETA, cor, tamanho, modifier)
