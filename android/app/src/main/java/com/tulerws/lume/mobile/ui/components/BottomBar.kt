package com.tulerws.lume.mobile.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.tulerws.lume.mobile.R
import com.tulerws.lume.mobile.ui.theme.LumeTheme

/** Os três destinos. Pareamento e detalhe de sessão não são destinos. */
enum class Destino { Sessoes, Historico, Ajustes }

/** Altura da pílula. O inset de gestos fica **por fora** dela. */
private val AlturaDaPilula = 64.dp

/** Folga entre a pílula e o inset de gestos. */
private val MargemInferior = 12.dp

/** Margem de tela, a mesma do documento. */
private val MargemLateral = 16.dp

/**
 * Espaço que uma lista precisa reservar no fim.
 *
 * A barra flutua **sobre** o conteúdo, então a última linha ficaria escondida sob
 * ela. E "a última sessão sumiu" parece defeito de dados, não de layout — por isso
 * o cálculo mora aqui, junto de quem conhece a geometria, e não espalhado em
 * números mágicos dentro de cada tela.
 */
@Composable
fun espacoParaBarra(): PaddingValues = PaddingValues(
    bottom = AlturaDaPilula + MargemInferior * 2 +
        WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding(),
)

/**
 * Barra inferior flutuante.
 *
 * ## Por que flutuante, e o que isso conserta
 *
 * A versão anterior aplicava `navigationBarsPadding()` **dentro** de uma altura de
 * 64dp, então o inset de gestos era descontado do conteúdo em vez de somado à
 * barra. Num aparelho com navegação por gestos sobravam ~20dp, e o rótulo era o
 * primeiro a ser cortado — foi o defeito visto no Galaxy M13.
 *
 * Flutuando, o problema deixa de existir por construção: a pílula tem altura
 * própria e o inset fica embaixo dela, onde `MOBILE-UI-DESIGN.md` sempre disse que
 * deveria estar ("64dp **mais** o inset").
 *
 * ## O item selecionado
 *
 * Círculo preenchido no acento, **sem rótulo** — o formato da referência que o
 * produto escolheu. Registro o custo, porque ele é real: o único destino cujo
 * nome não se lê passa a ser aquele em que o usuário está, e os nossos ícones são
 * abstratos (dois círculos, três linhas, uma engrenagem). O `contentDescription`
 * cobre o leitor de tela; quem enxerga depende do ícone.
 *
 * ## Elevação
 *
 * `MOBILE-UI-DESIGN.md` permite sombra na barra inferior, e só nela e na folha
 * modal, no valor do desktop. Uso esse valor — 1dp — em vez de inventar um maior
 * para "parecer flutuante". Se no aparelho a pílula não ler como solta, este é o
 * único número a revisitar, e com evidência.
 */
@Composable
fun BottomBar(
    selecionado: Destino,
    aoSelecionar: (Destino) -> Unit,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier
            .fillMaxWidth()
            .navigationBarsPadding()
            .padding(horizontal = MargemLateral, vertical = MargemInferior),
    ) {
        Surface(
            shape = LumeTheme.shapes.pill,
            color = LumeTheme.colors.surfaceRaised,
            shadowElevation = 1.dp,
            tonalElevation = 0.dp,
            modifier = Modifier.fillMaxWidth().height(AlturaDaPilula),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                ItemDeDestino(
                    rotulo = stringResource(R.string.nav_sessoes),
                    selecionado = selecionado == Destino.Sessoes,
                    aoTocar = { aoSelecionar(Destino.Sessoes) },
                    modifier = Modifier.weight(1f),
                ) { cor, tamanho -> IconeSessoes(cor, tamanho = tamanho) }

                ItemDeDestino(
                    rotulo = stringResource(R.string.nav_historico),
                    selecionado = selecionado == Destino.Historico,
                    aoTocar = { aoSelecionar(Destino.Historico) },
                    modifier = Modifier.weight(1f),
                ) { cor, tamanho -> IconeHistorico(cor, tamanho = tamanho) }

                ItemDeDestino(
                    rotulo = stringResource(R.string.nav_ajustes),
                    selecionado = selecionado == Destino.Ajustes,
                    aoTocar = { aoSelecionar(Destino.Ajustes) },
                    modifier = Modifier.weight(1f),
                ) { cor, tamanho -> IconeAjustes(cor, tamanho = tamanho) }
            }
        }
    }
}

@Composable
private fun ItemDeDestino(
    rotulo: String,
    selecionado: Boolean,
    aoTocar: () -> Unit,
    modifier: Modifier = Modifier,
    icone: @Composable (Color, Dp) -> Unit,
) {
    Column(
        modifier
            .fillMaxWidth()
            .height(AlturaDaPilula)
            // O alvo de toque é a coluna inteira — 64dp de altura por ~110dp de
            // largura. O desenho visível é menor, e o documento manda o alvo ser
            // 48dp mesmo assim.
            .clickable(onClick = aoTocar),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        if (selecionado) {
            Box(
                Modifier
                    .size(48.dp)
                    .clip(CircleShape)
                    .background(LumeTheme.colors.accent),
                contentAlignment = Alignment.Center,
            ) {
                // Sobre o acento, o ícone usa a cor da superfície — é o par de
                // contraste que `onPrimary` já carrega no tema.
                icone(LumeTheme.colors.surface, 24.dp)
            }
        } else {
            icone(LumeTheme.colors.inkMuted, 22.dp)
            androidx.compose.foundation.layout.Spacer(Modifier.height(6.dp))
            Text(
                text = rotulo.uppercase(),
                style = LumeTheme.typography.overline,
                color = LumeTheme.colors.inkMuted,
                maxLines = 1,
            )
        }
    }
}
