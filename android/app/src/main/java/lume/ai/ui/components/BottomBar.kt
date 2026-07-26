package lume.ai.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import lume.ai.R
import lume.ai.ui.theme.LumeTheme

/** Os três destinos. Pareamento e detalhe de sessão não são destinos. */
enum class Destino { Sessoes, Historico, Ajustes }

/**
 * Barra inferior.
 *
 * Três destinos — o mínimo que justifica uma barra inferior. Pareamento é
 * abertura e detalhe de sessão é empilhamento; nenhum dos dois entra aqui.
 *
 * **Sem `NavigationBar` do Material.** O componente dele desenha uma pílula atrás
 * do item selecionado e usa a própria paleta de estados. Aqui o realce é cor mais
 * peso — acento e `overline` em 750 no selecionado, `inkMuted` em 650 no resto —,
 * exatamente como o desktop faz. É também a única superfície do aplicativo com
 * sombra, e no valor do desktop.
 *
 * `navigationBarsPadding` afasta do gesto do sistema: a barra de 64dp é conteúdo,
 * e o inset vem por cima dela. Sem isso, os três destinos ficam sob a barra de
 * gestos em aparelho sem botões.
 */
@Composable
fun BottomBar(
    selecionado: Destino,
    aoSelecionar: (Destino) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier
            .fillMaxWidth()
            .background(LumeTheme.colors.surface),
    ) {
        Divisor()
        Row(
            Modifier
                .fillMaxWidth()
                .height(64.dp)
                .navigationBarsPadding(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ItemDeDestino(
                rotulo = stringResource(R.string.nav_sessoes),
                selecionado = selecionado == Destino.Sessoes,
                aoTocar = { aoSelecionar(Destino.Sessoes) },
                modifier = Modifier.weight(1f),
            ) { cor -> IconeSessoes(cor) }

            ItemDeDestino(
                rotulo = stringResource(R.string.nav_historico),
                selecionado = selecionado == Destino.Historico,
                aoTocar = { aoSelecionar(Destino.Historico) },
                modifier = Modifier.weight(1f),
            ) { cor -> IconeHistorico(cor) }

            ItemDeDestino(
                rotulo = stringResource(R.string.nav_ajustes),
                selecionado = selecionado == Destino.Ajustes,
                aoTocar = { aoSelecionar(Destino.Ajustes) },
                modifier = Modifier.weight(1f),
            ) { cor -> IconeAjustes(cor) }
        }
    }
}

@Composable
private fun ItemDeDestino(
    rotulo: String,
    selecionado: Boolean,
    aoTocar: () -> Unit,
    modifier: Modifier = Modifier,
    icone: @Composable (Color) -> Unit,
) {
    val cor = if (selecionado) LumeTheme.colors.accent else LumeTheme.colors.inkMuted
    Column(
        modifier
            .fillMaxWidth()
            .height(64.dp)
            .clickable(onClick = aoTocar),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        icone(cor)
        androidx.compose.foundation.layout.Spacer(Modifier.height(6.dp))
        Text(
            text = rotulo.uppercase(),
            style = LumeTheme.typography.overline.copy(
                fontWeight = if (selecionado) {
                    LumeTheme.typography.overline.fontWeight
                } else {
                    LumeTheme.typography.label.fontWeight
                },
            ),
            color = cor,
        )
    }
}
