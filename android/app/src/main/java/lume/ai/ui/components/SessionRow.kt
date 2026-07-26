package lume.ai.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import lume.ai.domain.AgentSession
import lume.ai.ui.coresDoEstado
import lume.ai.ui.rotuloDoEstado
import lume.ai.ui.tempoRelativo
import lume.ai.ui.theme.LumeTheme

/**
 * Uma linha da lista de sessões.
 *
 * 72dp de altura, barra de estado de 3dp à esquerda na cor de **preenchimento**, e
 * o rótulo de estado na cor **sobre** — nunca a mesma das duas. Usar o
 * preenchimento no texto reprova o contraste, e é o erro que a separação de
 * `StateColor` existe para tornar difícil.
 *
 * A altura é fixa porque a lista precisa ser lida de relance, e linha que muda de
 * altura conforme o nome do projeto quebra esse relance. Nome e projeto truncam
 * em uma linha; o estado é curto por construção, já que vem do enum.
 */
@Composable
fun SessionRow(
    sessao: AgentSession,
    aoTocar: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val cores = coresDoEstado(sessao.status)
    Row(
        modifier
            .fillMaxWidth()
            .height(72.dp)
            .clickable(onClick = aoTocar),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            Modifier
                .width(3.dp)
                .fillMaxHeight()
                .background(cores.fill),
        )
        Column(
            Modifier
                .weight(1f)
                .padding(start = 13.dp, end = 16.dp),
            verticalArrangement = Arrangement.spacedBy(5.dp),
        ) {
            Row(
                verticalAlignment = Alignment.Bottom,
                horizontalArrangement = Arrangement.spacedBy(7.dp),
            ) {
                androidx.compose.material3.Text(
                    text = sessao.agentLabel,
                    style = LumeTheme.typography.titleSmall,
                    color = LumeTheme.colors.ink,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                androidx.compose.material3.Text(
                    text = "· ${sessao.project}",
                    style = LumeTheme.typography.label,
                    color = LumeTheme.colors.inkMuted,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Row(
                Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.Bottom,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                androidx.compose.material3.Text(
                    text = stringResource(rotuloDoEstado(sessao.status)).uppercase(),
                    style = LumeTheme.typography.overline,
                    color = cores.on,
                    maxLines = 1,
                )
                androidx.compose.material3.Text(
                    text = tempoRelativo(sessao.updatedAt),
                    style = LumeTheme.typography.label,
                    color = LumeTheme.colors.inkMuted,
                    maxLines = 1,
                )
            }
        }
    }
}
