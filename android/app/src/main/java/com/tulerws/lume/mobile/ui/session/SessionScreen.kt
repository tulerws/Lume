package com.tulerws.lume.mobile.ui.session

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
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
import com.tulerws.lume.mobile.R
import com.tulerws.lume.mobile.domain.AgentSession
import com.tulerws.lume.mobile.domain.PermissionAction
import com.tulerws.lume.mobile.domain.SessionActivity
import com.tulerws.lume.mobile.domain.SessionStatus
import com.tulerws.lume.mobile.domain.abreTerminalNaMaquina
import com.tulerws.lume.mobile.domain.aceitaPrompt
import com.tulerws.lume.mobile.domain.acoesRespondiveis
import com.tulerws.lume.mobile.ui.components.BlocoMono
import com.tulerws.lume.mobile.ui.components.Divisor
import com.tulerws.lume.mobile.ui.components.IconeVoltar
import com.tulerws.lume.mobile.ui.components.PermissaoRespondida
import com.tulerws.lume.mobile.ui.components.PermissionBlock
import com.tulerws.lume.mobile.ui.components.PromptField
import com.tulerws.lume.mobile.ui.components.RotuloDeSecao
import com.tulerws.lume.mobile.ui.coresDaAtividade
import com.tulerws.lume.mobile.ui.horaDoRelogio
import com.tulerws.lume.mobile.ui.rotuloDaFalha
import com.tulerws.lume.mobile.ui.theme.LumeTheme

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
    /**
     * A sessão saiu do snapshot enquanto esta tela estava aberta.
     *
     * O conteúdo continua na tela — é o que o usuário estava lendo — e uma faixa
     * diz o que houve. Mesmo tratamento que o `permission_gone` já recebe no
     * protocolo: informação, não erro.
     */
    encerrada: Boolean = false,
    permissaoResolvida: PermissaoResolvida?,
    /**
     * Código da última ação que falhou, ou `null`.
     *
     * Aparece **onde o dedo estava** — abaixo do bloco de permissão, acima do
     * campo de prompt — e não numa faixa no topo. As duas falhas acontecem em
     * lugares diferentes da mesma tela, e ali o contexto é a posição.
     */
    falhaDaAcao: String?,
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

        if (encerrada) {
            FaixaDeSessaoEncerrada()
            Divisor()
        }

        val pendente = sessao.pendingPermission
        when {
            pendente != null -> {
                PermissionBlock(
                    pedido = pendente,
                    acoes = sessao.acoesRespondiveis(),
                    caminho = sessao.workingDirectory,
                    aoResponder = aoResponderPermissao,
                )
                if (falhaDaAcao != null) MensagemDeFalha(falhaDaAcao)
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
            if (falhaDaAcao != null && sessao.pendingPermission == null) {
                MensagemDeFalha(falhaDaAcao)
            }
            PromptField(
                texto = prompt,
                aoMudar = aoMudarPrompt,
                aoEnviar = aoEnviarPrompt,
                // Sessão encerrada não recebe prompt: o agente não está mais lá.
                habilitado = !encerrada && sessao.aceitaPrompt(),
                dica = motivoOuDica(sessao, encerrada),
                avisoDeTerminal = sessao.abreTerminalNaMaquina(),
            )
        }
    }
}

/**
 * O que deu errado na última ação, junto de onde ela foi tentada.
 *
 * Persistente até a ação seguinte dar certo. Um aviso que some sozinho serve para
 * confirmar sucesso, não para relatar que o portão continua fechado.
 */
@Composable
private fun MensagemDeFalha(codigo: String) {
    Text(
        text = stringResource(rotuloDaFalha(codigo)),
        style = LumeTheme.typography.label,
        color = LumeTheme.states.failed.on,
        modifier = Modifier
            .fillMaxWidth()
            .background(LumeTheme.colors.surfaceRaised)
            .padding(horizontal = 16.dp, vertical = 10.dp),
    )
}

/**
 * A sessão terminou enquanto a tela estava aberta.
 *
 * Fica logo abaixo do cabeçalho, acima de tudo que possa ser lido — quem chega
 * agora precisa saber que o que está vendo é final antes de tentar responder.
 */
@Composable
private fun FaixaDeSessaoEncerrada() {
    Row(
        Modifier
            .fillMaxWidth()
            .background(LumeTheme.colors.surfaceRaised)
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        Box(
            Modifier
                .size(8.dp)
                .clip(LumeTheme.shapes.marker)
                .background(LumeTheme.states.idle.fill),
        )
        Text(
            text = stringResource(R.string.sessao_encerrada),
            style = LumeTheme.typography.label,
            color = LumeTheme.colors.inkMuted,
        )
    }
}

/**
 * O texto dentro do campo: a dica quando dá para escrever, o **motivo** quando não
 * dá. Campo desabilitado sem explicação é interface muda.
 *
 * A recusa permanente vem **antes** do estado, e a ordem é a informação: uma
 * sessão sem retomada pode estar executando, e dizer "aguarde o agente terminar"
 * prometeria que esperar resolve. Não resolve — nunca vai aceitar prompt.
 */
@Composable
private fun motivoOuDica(sessao: AgentSession, encerrada: Boolean = false): String = when {
    encerrada -> stringResource(R.string.sessao_encerrada)
    !sessao.acceptsPrompt -> stringResource(R.string.prompt_desabilitado_sem_retomada)
    else -> when (sessao.status) {
        SessionStatus.Running -> stringResource(R.string.prompt_desabilitado_executando)
        SessionStatus.PermissionRequired -> stringResource(R.string.prompt_desabilitado_permissao)
        else -> stringResource(R.string.prompt_dica, sessao.agentLabel)
    }
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
                .clip(LumeTheme.shapes.marker)
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
