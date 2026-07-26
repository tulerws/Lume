package lume.ai.ui.pair

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
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
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import lume.ai.R
import lume.ai.data.EstadoDePareamento
import lume.ai.data.remote.FalhaDeLeitura
import lume.ai.ui.components.BotaoContornado
import lume.ai.ui.components.IconeFechar
import lume.ai.ui.theme.Brand
import lume.ai.ui.theme.LumeTheme

/**
 * Tela de pareamento.
 *
 * O primeiro gesto do produto. A câmera ocupa a tela, o enquadramento de 240dp
 * fica no centro, e **"Digitar endereço" está sempre visível** — ele é o que
 * resolve `h=` vazio e IP inalcançável, e esconder o caminho manual atrás de uma
 * falha seria esconder o que sobra quando o mDNS não passa.
 *
 * A tela ignora o tema e é escura sempre: o enquadramento precisa de contraste
 * contra a imagem da câmera, e a imagem não sabe qual tema o usuário escolheu.
 *
 * A permissão é pedida **aqui**, e não na abertura. Um pedido de câmera sem uma
 * câmera na tela é um pedido sem motivo aparente, e o usuário nega.
 */
@Composable
fun PairScreen(
    estado: EstadoDePareamento,
    falhaDeLeitura: FalhaDeLeitura?,
    aoLerQr: (String) -> Unit,
    aoFechar: () -> Unit,
    aoDigitarEndereco: () -> Unit,
    aoConcluir: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val contexto = LocalContext.current
    var temPermissao by remember {
        mutableStateOf(
            ContextCompat.checkSelfPermission(contexto, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED,
        )
    }
    var foiNegada by remember { mutableStateOf(false) }

    val pedido = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { concedida ->
        temPermissao = concedida
        foiNegada = !concedida
    }

    // Pareou: sai da tela. O `LaunchedEffect` reage à mudança de estado em vez de
    // a tela precisar saber quando checar.
    LaunchedEffect(estado) {
        if (estado == EstadoDePareamento.Concluido) aoConcluir()
    }

    Box(
        modifier
            .fillMaxSize()
            .background(gradienteDeFundo()),
    ) {
        if (temPermissao) {
            LeitorDeQr(aoLer = aoLerQr, modifier = Modifier.fillMaxSize())
        }

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
                    text = mensagemDoTopo(estado, falhaDeLeitura, temPermissao, foiNegada),
                    style = LumeTheme.typography.body,
                    color = Brand.Light,
                    textAlign = TextAlign.Center,
                )

                Spacer(Modifier.height(28.dp))
                Enquadramento()
                Spacer(Modifier.height(28.dp))

                if (!temPermissao) {
                    BotaoContornado(
                        texto = stringResource(R.string.pareamento_permissao_botao),
                        aoTocar = { pedido.launch(Manifest.permission.CAMERA) },
                        modifier = Modifier.fillMaxWidth(0.7f),
                    )
                    Spacer(Modifier.height(16.dp))
                }

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

                // Ler o QR é obrigatório para parear. Sem esta linha, "Digitar
                // endereço" passa por alternativa de pareamento — e não é uma.
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
 * O que a linha de cima diz, em ordem de urgência.
 *
 * Uma frase só, e a mais relevante agora. Empilhar instrução, erro de leitura e
 * estado de conexão faria a tela mais cheia justamente no momento em que o
 * usuário está segurando o celular contra um monitor.
 */
@Composable
private fun mensagemDoTopo(
    estado: EstadoDePareamento,
    falha: FalhaDeLeitura?,
    temPermissao: Boolean,
    foiNegada: Boolean,
): String = when {
    estado is EstadoDePareamento.EmAndamento ->
        stringResource(R.string.pareamento_conectando, "")
    estado is EstadoDePareamento.Falhou -> estado.motivo
    foiNegada -> stringResource(R.string.pareamento_permissao_negada)
    !temPermissao -> stringResource(R.string.pareamento_permissao_corpo)
    falha is FalhaDeLeitura.VersaoDesconhecida -> stringResource(R.string.pareamento_erro_versao)
    falha is FalhaDeLeitura.CampoCorrompido || falha is FalhaDeLeitura.CampoAusente ->
        stringResource(R.string.pareamento_erro_corrompido)
    else -> stringResource(R.string.pareamento_instrucao)
}

/**
 * O gradiente do design.
 *
 * Fica **atrás** da câmera e aparece antes de o primeiro quadro chegar, e no
 * lugar dela quando não há permissão. Sem ele a tela pisca preto na abertura.
 */
private fun gradienteDeFundo() = Brush.radialGradient(
    colorStops = arrayOf(
        0f to Color(0xFF1E2A25),
        0.55f to Color(0xFF111815),
        1f to Color(0xFF0A0F0D),
    ),
    center = Offset.Unspecified,
    radius = Float.POSITIVE_INFINITY,
)

/**
 * Os quatro cantos de leitura, 240dp.
 *
 * Cantos em vez de moldura fechada: a moldura inteira competiria com o QR pela
 * atenção da câmera e do olho. É o mesmo canto rotacionado quatro vezes — mesma
 * arte, quatro orientações, nenhuma chance de discordarem entre si.
 */
@Composable
private fun Enquadramento() {
    val canto: Path = remember { PathParser().parsePathString("M0,0h10v3H0zM0,0h3v10H0z").toPath() }
    Canvas(Modifier.size(240.dp)) {
        val lado = 40.dp.toPx()
        val escala = lado / 10f
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
