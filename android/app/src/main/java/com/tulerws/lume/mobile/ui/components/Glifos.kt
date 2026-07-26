package com.tulerws.lume.mobile.ui.components

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
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.tulerws.lume.mobile.ui.theme.LumeTheme

/**
 * Ícones, em três famílias.
 *
 * · **Glifos de estado** — tique e exclamação, grade de 6. Pixel, porque vivem no
 *   mascote, que é pixel. São os mesmos do `LumeMascot.svelte`.
 *
 * · **Destinos da barra inferior** — grade de 20, **traçados**, medidos do rodapé
 *   do desktop. `docs/MOBILE-UI-DESIGN.md` mandava grade de pixel também aqui,
 *   mas aquela regra foi escrita a partir do *logo*: o rodapé do próprio desktop
 *   usa traço, e para uma barra de navegação a referência certa é a barra de
 *   navegação, não a marca. A linha do documento está registrada como pendente.
 *
 * · **Afordâncias universais** — voltar, fechar, enviar. Traço do Material, grade
 *   de 24. Desenhá-las em pixel prejudicaria o reconhecimento sem ganhar
 *   identidade: ninguém precisa de uma seta de voltar autoral.
 *
 * Nenhum caminho aqui foi redesenhado. Todos vieram medidos da origem.
 */

private object Caminhos {
    private val cache = mutableMapOf<String, Path>()
    fun de(dados: String): Path = cache.getOrPut(dados) { PathParser().parsePathString(dados).toPath() }
}

// Grade de 20, traçados, vindos do rodapé do desktop (`+page.svelte:2750-2795`).
//
// Foram medidos de lá, não redesenhados: `fill: none`, `stroke: currentColor`,
// espessura 1,65 e pontas arredondadas. Substituíram a grade de pixel porque a
// referência certa para uma barra de navegação é a barra de navegação do
// desktop — a regra de pixel do documento tinha sido escrita a partir do logo.
private const val AJUSTES_RAIOS =
    "M10,2.5v2M10,15.5v2M2.5,10h2M15.5,10h2M4.7,4.7l1.4,1.4M13.9,13.9l1.4,1.4M15.3,4.7l-1.4,1.4M6.1,13.9l-1.4,1.4"
private const val HISTORICO_LINHAS = "M4.5,5.5h11M4.5,10h11M4.5,14.5h7"

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

/**
 * Um ícone de destino, na grade de 20 do desktop.
 *
 * A espessura de 1,65 é escalada junto com o desenho: fixá-la em `dp` faria o
 * traço engrossar em relação à arte a cada mudança de tamanho, que é como um
 * conjunto de ícones deixa de parecer um conjunto.
 */
@Composable
private fun IconeDeDestino(
    cor: Color,
    tamanho: Dp,
    modifier: Modifier,
    desenho: DrawScope.(escala: Float) -> Unit,
) {
    Canvas(modifier.size(tamanho).clearAndSetSemantics { }) {
        desenho(size.width / 20f)
    }
}

private fun DrawScope.tracoDe(escala: Float) =
    Stroke(width = 1.65f * escala, cap = StrokeCap.Round, join = StrokeJoin.Round)

@Composable
fun IconeSessoes(cor: Color, modifier: Modifier = Modifier, tamanho: Dp = 22.dp) =
    IconeDeDestino(cor, tamanho, modifier) { escala ->
        // Duas sessões lado a lado. É o ícone do desktop, sem redesenho.
        listOf(6f, 14f).forEach { x ->
            drawCircle(cor, radius = 2.5f * escala, center = Offset(x * escala, 10f * escala), style = tracoDe(escala))
        }
    }

@Composable
fun IconeHistorico(cor: Color, modifier: Modifier = Modifier, tamanho: Dp = 22.dp) =
    IconeDeDestino(cor, tamanho, modifier) { escala ->
        withTransform({ scale(escala, escala, Offset.Zero) }) {
            drawPath(Caminhos.de(HISTORICO_LINHAS), cor, style = Stroke(1.65f, cap = StrokeCap.Round, join = StrokeJoin.Round))
        }
    }

@Composable
fun IconeAjustes(cor: Color, modifier: Modifier = Modifier, tamanho: Dp = 22.dp) =
    IconeDeDestino(cor, tamanho, modifier) { escala ->
        drawCircle(cor, radius = 3f * escala, center = Offset(10f * escala, 10f * escala), style = tracoDe(escala))
        withTransform({ scale(escala, escala, Offset.Zero) }) {
            drawPath(Caminhos.de(AJUSTES_RAIOS), cor, style = Stroke(1.65f, cap = StrokeCap.Round, join = StrokeJoin.Round))
        }
    }

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
