package lume.ai.ui.pair

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import lume.ai.R
import lume.ai.ui.components.IconeFechar
import lume.ai.ui.theme.Brand
import lume.ai.ui.theme.LumeTheme

/**
 * Tela de pareamento.
 *
 * **Não há câmera aqui.** CameraX e ML Kit ficaram fora deste trabalho por
 * decisão explícita; o que existe é o enquadramento do design sobre o gradiente
 * escuro, na posição e no tamanho em que a pré-visualização vai entrar. Quando a
 * câmera chegar, ela ocupa o fundo e o resto desta tela não muda.
 *
 * O caminho manual fica **sempre visível**, e não escondido atrás de uma falha de
 * leitura. Ele é o que faz VPN e redirecionamento de porta funcionarem, é o que
 * resta quando o mDNS não passa, e é o caminho de quem usa leitor de tela.
 *
 * A tela ignora o tema: é escura sempre. O enquadramento precisa de contraste
 * contra a imagem da câmera, e a imagem da câmera não sabe qual tema o usuário
 * escolheu.
 */
@Composable
fun PairScreen(
    aoFechar: () -> Unit,
    aoDigitarEndereco: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier
            .fillMaxSize()
            .background(
                // O gradiente do design: claro no alto e ao centro, escurecendo
                // para as bordas. O centro em 34% da altura põe a luz atrás do
                // enquadramento, não atrás do texto.
                Brush.radialGradient(
                    colorStops = arrayOf(
                        0f to Color(0xFF1E2A25),
                        0.55f to Color(0xFF111815),
                        1f to Color(0xFF0A0F0D),
                    ),
                    center = Offset.Unspecified,
                    radius = Float.POSITIVE_INFINITY,
                ),
            ),
    ) {
        Column(Modifier.fillMaxSize().statusBarsPadding()) {
            val rotuloFechar = stringResource(R.string.acao_fechar)
            Box(
                Modifier
                    .padding(horizontal = 8.dp)
                    .size(56.dp)
                    .clickable(onClick = aoFechar)
                    .semantics {
                        contentDescription = rotuloFechar
                        role = Role.Button
                    },
                contentAlignment = Alignment.Center,
            ) {
                IconeFechar(Brand.Light)
            }

            Column(
                Modifier
                    .weight(1f)
                    .fillMaxWidth()
                    .padding(horizontal = 24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
            ) {
                Text(
                    text = stringResource(R.string.pareamento_instrucao),
                    style = LumeTheme.typography.body,
                    color = Brand.Light,
                    textAlign = TextAlign.Center,
                )
                Spacer(Modifier.height(28.dp))
                Enquadramento()
                Spacer(Modifier.height(28.dp))
                Box(
                    Modifier
                        .height(48.dp)
                        .clickable(onClick = aoDigitarEndereco)
                        .padding(horizontal = 16.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = stringResource(R.string.pareamento_manual),
                        style = LumeTheme.typography.button,
                        color = Brand.Highlight,
                    )
                }
                // A tela precisa **dizer** que ler o QR é obrigatório, em vez de
                // deixar "Digitar endereço" passar por alternativa de pareamento —
                // ele não completa pareamento nenhum, e a exigência da câmera é
                // limitação assumida da v1, não descuido de interface.
                Text(
                    text = stringResource(R.string.pareamento_exige_qr),
                    style = LumeTheme.typography.label,
                    color = Brand.Light.copy(alpha = 0.7f),
                    textAlign = TextAlign.Center,
                    modifier = Modifier.padding(horizontal = 8.dp),
                )
            }
            Spacer(Modifier.navigationBarsPadding().height(24.dp))
        }
    }
}

/**
 * Os quatro cantos de leitura, 240dp.
 *
 * Cantos em vez de moldura fechada: a moldura inteira competiria com o QR pela
 * atenção da câmera e do olho. O desenho é o mesmo canto rotacionado quatro
 * vezes — mesma arte, quatro orientações, nenhuma chance de os cantos discordarem
 * entre si.
 */
@Composable
private fun Enquadramento() {
    val canto: Path = remember { PathParser().parsePathString("M0,0h10v3H0zM0,0h3v10H0z").toPath() }
    Canvas(Modifier.size(240.dp)) {
        val lado = 40.dp.toPx()
        val escala = lado / 10f
        // (deslocamento X, deslocamento Y, rotação) de cada canto.
        val posicoes = listOf(
            Triple(0f, 0f, 0f),
            Triple(size.width, 0f, 90f),
            Triple(size.width, size.height, 180f),
            Triple(0f, size.height, 270f),
        )
        posicoes.forEach { (x, y, giro) ->
            withTransform({
                translate(x, y)
                rotate(giro, Offset.Zero)
                scale(escala, escala, Offset.Zero)
            }) {
                drawPath(canto, Brand.Body)
            }
        }
    }
}
