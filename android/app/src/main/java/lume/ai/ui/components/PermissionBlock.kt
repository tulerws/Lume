package lume.ai.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.material3.Text
import kotlinx.coroutines.delay
import lume.ai.R
import lume.ai.domain.PermissionAction
import lume.ai.domain.PermissionRequest
import lume.ai.domain.RiskLevel
import lume.ai.ui.coresDoRisco
import lume.ai.ui.rotuloDaAcao
import lume.ai.ui.rotuloDoRisco
import lume.ai.ui.theme.LumeTheme

/**
 * O bloco de permissão — o componente mais importante do produto.
 *
 * É a razão de o aplicativo existir: destravar uma sessão parada esperando
 * resposta, com o usuário longe do computador.
 *
 * Regras que não são negociáveis, e que estão implementadas aqui e não na tela:
 *
 * · **O comando aparece antes dos botões.** Aprovar o que não se leu é o modo de
 *   falha que o produto existe para evitar, e ordem de leitura é o que impede.
 *
 * · **Recusar fica separado**, com 16dp de folga e tratamento de texto — nunca
 *   adjacente e idêntico às aprovações. Toque errado ali aprova execução
 *   destrutiva.
 *
 * · **Só ações de [acoes] são desenhadas.** A lista já vem filtrada por
 *   `AgentSession.acoesRespondiveis()`, que aplica `availableActions` **e**
 *   `canRespondFromLume`. Lista vazia desenha o aviso de somente observação — e
 *   não um bloco sem botão, que pareceria defeito.
 *
 * · **Risco alto pede segundo toque.** O botão vira "Confirmar permitir" por
 *   [SEGUNDOS_DE_CONFIRMACAO] segundos e muda de cor. Atrito deliberado, e só
 *   onde o dano é irreversível — em risco médio ou baixo isso seria só estorvo.
 */
private const val SEGUNDOS_DE_CONFIRMACAO = 3

@Composable
fun PermissionBlock(
    pedido: PermissionRequest,
    acoes: List<PermissionAction>,
    caminho: String?,
    aoResponder: (PermissionAction) -> Unit,
    modifier: Modifier = Modifier,
) {
    val risco = coresDoRisco(pedido.risk)

    // A confirmação vive aqui, e não no ViewModel, de propósito: ela é estado de
    // um gesto em andamento, não do domínio. Sair da tela e voltar deve pedir o
    // primeiro toque de novo — o que acontece naturalmente quando o estado morre
    // com o componente.
    var confirmando by remember(pedido.id) { mutableIntStateOf(0) }

    if (confirmando > 0) {
        LaunchedEffect(pedido.id, confirmando) {
            delay(1_000)
            confirmando -= 1
        }
    }

    Column(
        modifier
            .fillMaxWidth()
            .background(LumeTheme.colors.surfaceRaised)
            .padding(16.dp),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Box(
                Modifier
                    .width(3.dp)
                    .height(12.dp)
                    .background(risco.fill),
            )
            Text(
                text = stringResource(rotuloDoRisco(pedido.risk)).uppercase(),
                style = LumeTheme.typography.overline,
                color = risco.on,
            )
        }

        Spacer(Modifier.height(12.dp))
        Text(
            text = pedido.summary,
            style = LumeTheme.typography.title,
            color = LumeTheme.colors.ink,
        )

        Spacer(Modifier.height(10.dp))
        BlocoMono(pedido.resource, Modifier.fillMaxWidth())

        if (caminho != null) {
            Spacer(Modifier.height(8.dp))
            Text(
                text = caminho,
                style = LumeTheme.typography.label,
                color = LumeTheme.colors.inkMuted,
            )
        }

        if (acoes.isEmpty()) {
            // `canRespondFromLume` falso, ou nenhuma ação disponível. O celular
            // não ganha poder que o desktop não tem, e dizer isso é melhor do que
            // mostrar um bloco truncado.
            Spacer(Modifier.height(16.dp))
            Text(
                text = stringResource(R.string.permissao_somente_observacao),
                style = LumeTheme.typography.body,
                color = LumeTheme.colors.inkMuted,
            )
            return@Column
        }

        Spacer(Modifier.height(16.dp))
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (PermissionAction.AllowOnce in acoes) {
                val pedindoConfirmacao = pedido.risk == RiskLevel.High && confirmando > 0
                BotaoPreenchido(
                    texto = if (pedindoConfirmacao) {
                        stringResource(R.string.permissao_confirmar)
                    } else {
                        stringResource(rotuloDaAcao(PermissionAction.AllowOnce))
                    },
                    sufixo = if (pedindoConfirmacao) {
                        stringResource(R.string.permissao_confirmar_contagem, confirmando)
                    } else {
                        null
                    },
                    corDeFundo = if (pedindoConfirmacao) risco.on else LumeTheme.colors.accent,
                    aoTocar = {
                        when {
                            pedido.risk != RiskLevel.High -> aoResponder(PermissionAction.AllowOnce)
                            confirmando > 0 -> aoResponder(PermissionAction.AllowOnce)
                            else -> confirmando = SEGUNDOS_DE_CONFIRMACAO
                        }
                    },
                )
            }
            if (PermissionAction.AllowSession in acoes) {
                BotaoContornado(
                    texto = stringResource(rotuloDaAcao(PermissionAction.AllowSession)),
                    aoTocar = { aoResponder(PermissionAction.AllowSession) },
                )
            }
        }

        if (PermissionAction.Deny in acoes) {
            Spacer(Modifier.height(16.dp))
            BotaoDeTexto(
                texto = stringResource(rotuloDaAcao(PermissionAction.Deny)),
                aoTocar = { aoResponder(PermissionAction.Deny) },
            )
        }
    }
}

/**
 * O bloco depois de a permissão ter sumido.
 *
 * Permissão respondida em outro lugar é **informação, não erro**: o protocolo
 * chama isso de `permission_gone` e a interface trata como situação normal. Trocar
 * o bloco por uma linha explicando é melhor do que fazer os botões sumirem, que
 * pareceria falha de carregamento.
 */
@Composable
fun PermissaoRespondida(
    titulo: String,
    detalhe: String,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier
            .fillMaxWidth()
            .background(LumeTheme.colors.surfaceRaised)
            .padding(16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        GlifoDeConfirmacao(Modifier.size(20.dp))
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(
                text = titulo,
                style = LumeTheme.typography.body.copy(
                    fontWeight = LumeTheme.typography.titleSmall.fontWeight,
                ),
                color = LumeTheme.colors.ink,
            )
            Text(
                text = detalhe,
                style = LumeTheme.typography.label,
                color = LumeTheme.colors.inkMuted,
            )
        }
    }
}
