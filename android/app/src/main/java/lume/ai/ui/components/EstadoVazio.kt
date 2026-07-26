package lume.ai.ui.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import lume.ai.ui.theme.LumeTheme

/**
 * Estado vazio ou de erro, centralizado.
 *
 * O mesmo componente serve aos dois porque a estrutura é a mesma — marca, título,
 * explicação e, às vezes, uma saída. O que muda é o tom, e o tom está no texto:
 * tela vazia convida ("Assim que um começar, ele aparece aqui"), erro diz o que
 * houve e o que fazer, sem pedir desculpa.
 *
 * O deslocamento de 60dp para cima não é capricho: centralizado de verdade, o
 * bloco fica visualmente baixo demais, porque a barra inferior já ocupa a base da
 * tela e o olho lê o centro do espaço livre, não o centro da tela.
 */
@Composable
fun EstadoVazio(
    titulo: String,
    corpo: String,
    modifier: Modifier = Modifier,
    marca: @Composable () -> Unit = {},
    acao: @Composable () -> Unit = {},
) {
    Column(
        modifier
            .fillMaxSize()
            .padding(horizontal = 32.dp)
            .padding(bottom = 60.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        marca()
        Spacer(Modifier.height(16.dp))
        Text(
            text = titulo,
            style = LumeTheme.typography.display,
            color = LumeTheme.colors.ink,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(14.dp))
        Text(
            text = corpo,
            style = LumeTheme.typography.body,
            color = LumeTheme.colors.inkMuted,
            textAlign = TextAlign.Center,
        )
        acao()
    }
}
