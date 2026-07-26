package lume.ai.ui.session

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
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import lume.ai.R
import lume.ai.domain.AgentSession
import lume.ai.domain.PermissionAction
import lume.ai.domain.SessionActivity
import lume.ai.domain.SessionStatus
import lume.ai.domain.abreTerminalNaMaquina
import lume.ai.domain.aceitaPrompt
import lume.ai.domain.acoesRespondiveis
import lume.ai.ui.components.BlocoMono
import lume.ai.ui.components.Divisor
import lume.ai.ui.components.IconeVoltar
import lume.ai.ui.components.PermissaoRespondida
import lume.ai.ui.components.PermissionBlock
import lume.ai.ui.components.PromptField
import lume.ai.ui.components.RotuloDeSecao
import lume.ai.ui.coresDaAtividade
import lume.ai.ui.horaDoRelogio
import lume.ai.ui.theme.LumeTheme

/**
 * Resposta que já aconteceu, para o cartão que substitui o bloco de permissão.
 *
 * Carrega a **ação**, não o rótulo dela. Texto que o usuário lê nasce de
 * `strings.xml`, e um `String` aqui traria a tradução para dentro do ViewModel —
 * onde não há `Context`, não há locale e a tela em inglês receberia português.
 *
 * [emOutroDispositivo] separa dois casos que a interface precisa contar de forma
 * diferente. Se o usuário respondeu aqui, dizer "respondida em outro dispositivo"
 * seria mentira. Se a permissão sumiu sozinha, o aplicativo **não sabe** qual ação
 * foi tomada — sabe só que não é mais dele.
 */
data class PermissaoResolvida(
    val acao: PermissionAction,
    val horario: String,
    val emOutroDispositivo: Boolean,
)

/**
 * Tela de Sessão.
 *
 * O bloco de permissão fica **fixo no topo**, nunca abaixo da dobra: é a razão de
 * o usuário ter aberto o aplicativo, e rolar para encontrá-lo seria enterrar o
 * produto dentro do produto. A atividade rola por baixo dele.
 */
@Composable
fun SessionScreen(
    sessao: AgentSession,
    permissaoResolvida: PermissaoResolvida?,
    prompt: String,
    aoMudarPrompt: (String) -> Unit,
    aoEnviarPrompt: () -> Unit,
    aoResponderPermissao: (PermissionAction) -> Unit,
    aoVoltar: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier
            .fillMaxSize()
            .background(LumeTheme.colors.surface),
    ) {
        Cabecalho(sessao, aoVoltar, Modifier.statusBarsPadding())

        val pendente = sessao.pendingPermission
        when {
            pendente != null -> {
                PermissionBlock(
                    pedido = pendente,
                    acoes = sessao.acoesRespondiveis(),
                    caminho = sessao.workingDirectory,
                    aoResponder = aoResponderPermissao,
                )
                Divisor()
            }

            permissaoResolvida != null -> {
                val resultado = stringResource(
                    if (permissaoResolvida.acao == PermissionAction.Deny) {
                        R.string.permissao_recusada
                    } else {
                        R.string.permissao_concedida
                    },
                )
                PermissaoRespondida(
                    titulo = if (permissaoResolvida.emOutroDispositivo) {
                        stringResource(R.string.permissao_respondida_titulo)
                    } else {
                        resultado
                    },
                    detalhe = "$resultado · ${permissaoResolvida.horario}",
                )
                Divisor()
            }
        }

        LazyColumn(
            Modifier
                .weight(1f)
                .fillMaxWidth()
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            item {
                RotuloDeSecao(stringResource(R.string.sessao_atividade))
            }
            items(sessao.activities, key = { it.id }) { atividade ->
                LinhaDeAtividade(atividade)
            }
        }

        Column(
            Modifier
                .background(LumeTheme.colors.surface)
                .imePadding()
                .navigationBarsPadding(),
        ) {
            Divisor()
            PromptField(
                texto = prompt,
                aoMudar = aoMudarPrompt,
                aoEnviar = aoEnviarPrompt,
                habilitado = sessao.aceitaPrompt(),
                dica = motivoOuDica(sessao),
                avisoDeTerminal = sessao.abreTerminalNaMaquina(),
            )
        }
    }
}

/**
 * O texto dentro do campo: a dica quando dá para escrever, o **motivo** quando não
 * dá. Campo desabilitado sem explicação é interface muda.
 */
@Composable
private fun motivoOuDica(sessao: AgentSession): String = when (sessao.status) {
    SessionStatus.Running -> stringResource(R.string.prompt_desabilitado_executando)
    SessionStatus.PermissionRequired -> stringResource(R.string.prompt_desabilitado_permissao)
    else -> stringResource(R.string.prompt_dica, sessao.agentLabel)
}

@Composable
private fun Cabecalho(sessao: AgentSession, aoVoltar: () -> Unit, modifier: Modifier = Modifier) {
    Column(modifier) {
        Row(
            Modifier
                .fillMaxWidth()
                .height(56.dp)
                .padding(horizontal = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            val rotuloVoltar = stringResource(R.string.acao_voltar)
            Box(
                Modifier
                    // 48dp de alvo em torno de um desenho de 24dp: o mínimo do
                    // documento vale mesmo quando a arte visível é menor.
                    .size(48.dp)
                    .clickable(onClick = aoVoltar)
                    // A descrição vive no alvo de toque, não no desenho: os glifos
                    // são `clearAndSetSemantics`, e é o alvo que o leitor de tela
                    // anuncia e o usuário aciona.
                    .semantics {
                        contentDescription = rotuloVoltar
                        role = Role.Button
                    },
                contentAlignment = Alignment.Center,
            ) {
                IconeVoltar(LumeTheme.colors.ink)
            }
            Column(verticalArrangement = Arrangement.spacedBy(1.dp)) {
                Text(
                    text = sessao.agentLabel,
                    style = LumeTheme.typography.titleSmall,
                    color = LumeTheme.colors.ink,
                )
                Text(
                    text = sessao.project,
                    style = LumeTheme.typography.label,
                    color = LumeTheme.colors.inkMuted,
                )
            }
        }
        Divisor()
    }
}

@Composable
private fun LinhaDeAtividade(atividade: SessionActivity) {
    Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        Box(
            Modifier
                // 6dp de topo alinha o ponto com a primeira linha do texto, e não
                // com o topo da caixa — que ficaria alto demais.
                .padding(top = 6.dp)
                .size(8.dp)
                .clip(RoundedCornerShape(4.dp))
                .background(coresDaAtividade(atividade.status).fill),
        )
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(
                text = atividade.title,
                style = LumeTheme.typography.body,
                color = LumeTheme.colors.ink,
            )
            atividade.detail?.let { detalhe ->
                BlocoMono(detalhe)
            }
            Text(
                text = horaDoRelogio(atividade.createdAt),
                style = LumeTheme.typography.label,
                color = LumeTheme.colors.inkMuted,
            )
        }
    }
}
