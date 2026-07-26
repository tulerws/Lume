package lume.ai.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import lume.ai.R
import lume.ai.domain.ConnectionState
import lume.ai.ui.coresDaConexao
import lume.ai.ui.rotuloDaConexao
import lume.ai.ui.theme.LumeTheme

/**
 * Peças reusadas por mais de uma tela.
 *
 * Nenhuma delas usa componente do Material com aparência própria — `Button`,
 * `Card`, `Divider`. Os do Material trazem elevação, preenchimento interno e
 * raio que não são os do design, e sobrescrever cada um deles em cada uso é mais
 * código e mais chance de esquecer. `docs/MOBILE-UI-DESIGN.md` é explícito: a
 * separação vem de borda de 1dp mais mudança de superfície, e a elevação é zero
 * em todo lugar exceto a barra inferior.
 */

@Composable
fun LumeLogo(modifier: Modifier = Modifier, tamanho: Dp = 26.dp) {
    androidx.compose.foundation.Image(
        painter = painterResource(R.drawable.ic_lume_logo),
        // Decorativo: o nome "Lume" está escrito ao lado em toda aparição.
        contentDescription = null,
        contentScale = ContentScale.Fit,
        modifier = modifier.size(tamanho),
    )
}

/**
 * Ponto e rótulo do estado da conexão.
 *
 * Os dois juntos, sempre. Cor nunca é o único portador de estado — quem não
 * distingue o verde do cinza lê "Conectado" ou "Sem conexão" do lado.
 */
@Composable
fun PontoDeConexao(estado: ConnectionState, modifier: Modifier = Modifier) {
    val cores = coresDaConexao(estado)
    Row(
        modifier = modifier,
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(7.dp),
    ) {
        Box(
            Modifier
                .size(8.dp)
                .clip(CircleShape)
                .background(cores.fill),
        )
        Text(
            text = stringResource(rotuloDaConexao(estado)),
            style = LumeTheme.typography.label,
            color = cores.on,
        )
    }
}

/** Rótulo de seção em caixa alta: "Aparelho", "Tema", "Atividade". */
@Composable
fun RotuloDeSecao(texto: String, modifier: Modifier = Modifier) {
    Text(
        text = texto.uppercase(),
        style = LumeTheme.typography.overline,
        color = LumeTheme.colors.inkMuted,
        modifier = modifier,
    )
}

/** Linha de 1dp na cor `line`. É toda a separação que o design usa. */
@Composable
fun Divisor(modifier: Modifier = Modifier) {
    Box(
        modifier
            .fillMaxWidth()
            .height(1.dp)
            .background(LumeTheme.colors.line),
    )
}

/** Cartão de seção: superfície elevada, borda de 1dp, raio de 14dp, sombra nenhuma. */
@Composable
fun Cartao(modifier: Modifier = Modifier, conteudo: @Composable () -> Unit) {
    Column(
        modifier
            .fillMaxWidth()
            .clip(LumeTheme.shapes.card)
            .background(LumeTheme.colors.surfaceRaised)
            .border(1.dp, LumeTheme.colors.line, LumeTheme.shapes.card),
    ) {
        conteudo()
    }
}

/**
 * Bloco monoespaçado: comando, caminho, saída de ferramenta.
 *
 * Texto que o agente vai executar precisa parecer executável. É a única coisa na
 * interface que não usa a Inter, e essa exceção é a informação.
 */
@Composable
fun BlocoMono(texto: String, modifier: Modifier = Modifier) {
    Text(
        text = texto,
        style = LumeTheme.typography.mono,
        color = LumeTheme.colors.codeInk,
        modifier = modifier
            .clip(LumeTheme.shapes.field)
            .background(LumeTheme.colors.codeSurface)
            .padding(horizontal = 14.dp, vertical = 12.dp),
    )
}

/**
 * Botão de 48dp.
 *
 * Três tratamentos, e a distância entre eles é deliberada. O preenchido é a ação
 * que se espera; o contornado é alternativa de mesmo peso; o de texto é para
 * quando adjacência com uma aprovação seria perigosa — ver o bloco de permissão.
 */
@Composable
fun BotaoPreenchido(
    texto: String,
    aoTocar: () -> Unit,
    modifier: Modifier = Modifier,
    corDeFundo: Color = LumeTheme.colors.accent,
    corDoTexto: Color = LumeTheme.colors.surface,
    sufixo: String? = null,
) {
    Row(
        modifier
            .fillMaxWidth()
            .height(48.dp)
            .clip(LumeTheme.shapes.field)
            .background(corDeFundo)
            .clickable(onClick = aoTocar),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(texto, style = LumeTheme.typography.button, color = corDoTexto)
        if (sufixo != null) {
            Text(
                text = "  $sufixo",
                style = LumeTheme.typography.label,
                color = corDoTexto.copy(alpha = 0.75f),
            )
        }
    }
}

@Composable
fun BotaoContornado(
    texto: String,
    aoTocar: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val acento = LumeTheme.colors.accent
    Box(
        modifier
            .fillMaxWidth()
            .height(48.dp)
            .clip(LumeTheme.shapes.field)
            // 55% de alfa: o contorno precisa ficar abaixo do preenchido na
            // hierarquia, e um contorno sólido na cor de acento competiria com ele.
            .border(1.dp, acento.copy(alpha = 0.55f), LumeTheme.shapes.field)
            .clickable(onClick = aoTocar),
        contentAlignment = Alignment.Center,
    ) {
        Text(texto, style = LumeTheme.typography.button, color = acento)
    }
}

@Composable
fun BotaoDeTexto(
    texto: String,
    aoTocar: () -> Unit,
    modifier: Modifier = Modifier,
    cor: Color = LumeTheme.colors.inkMuted,
) {
    Box(
        modifier
            .fillMaxWidth()
            .height(48.dp)
            .clip(RoundedCornerShape(10.dp))
            .clickable(onClick = aoTocar),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            texto,
            style = LumeTheme.typography.button.copy(fontWeight = LumeTheme.typography.label.fontWeight),
            color = cor,
        )
    }
}
