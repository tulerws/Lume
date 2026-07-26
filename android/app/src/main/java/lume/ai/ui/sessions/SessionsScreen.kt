package lume.ai.ui.sessions

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import lume.ai.R
import lume.ai.domain.AgentSession
import lume.ai.domain.ConnectionState
import lume.ai.ui.components.BotaoContornado
import lume.ai.ui.components.Divisor
import lume.ai.ui.components.EstadoVazio
import lume.ai.ui.components.GlifoDeAtencao
import lume.ai.ui.components.LumeLogo
import lume.ai.ui.components.LumeMascot
import lume.ai.ui.components.PontoDeConexao
import lume.ai.ui.components.SessionRow
import lume.ai.ui.estadoDoMascote
import lume.ai.ui.resumoDasSessoes
import lume.ai.ui.theme.LumeTheme

/**
 * Tela de Sessões.
 *
 * O estado da conexão aparece de forma **permanente**, no cabeçalho e no mascote.
 * Não é redundância: uma lista parada porque a conexão caiu não pode ter a mesma
 * aparência de uma lista de agentes parados, e essa é a confusão mais provável
 * desta interface. Sem conexão são três sinais para a mesma informação — faixa no
 * topo, mascote dormindo e cache a 60% —, porque o custo de o usuário errar aqui é
 * agir sobre dado velho.
 *
 * A ordenação **não** está aqui. Ela é do repositório, para que nenhuma tela possa
 * esquecê-la.
 */
@Composable
fun SessionsScreen(
    conexao: ConnectionState,
    sessoes: List<AgentSession>,
    aoAbrirSessao: (String) -> Unit,
    aoTentarNovamente: () -> Unit,
    nomeDoDesktop: String,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier
            .fillMaxSize()
            .background(LumeTheme.colors.surface),
    ) {
        // A faixa vem **acima** do cabeçalho, como no design. Abaixo dele, ela
        // acabaria rolando junto com o conteúdo em alguma refatoração futura — e o
        // aviso de que o dado está velho não pode sair da tela.
        if (conexao is ConnectionState.Desconectado) {
            FaixaDeDesconexao(nomeDoDesktop, Modifier.statusBarsPadding())
            Cabecalho(conexao)
        } else {
            Cabecalho(conexao, Modifier.statusBarsPadding())
        }

        when {
            conexao is ConnectionState.Erro -> EstadoDeErro(nomeDoDesktop, aoTentarNovamente)

            sessoes.isEmpty() -> EstadoVazio(
                titulo = stringResource(R.string.sessoes_vazio_titulo),
                corpo = stringResource(R.string.sessoes_vazio_corpo),
                marca = {
                    LumeMascot(estadoDoMascote(conexao, sessoes), largura = 96.dp)
                    Spacer(Modifier.height(8.dp))
                },
            )

            else -> {
                ResumoComMascote(conexao, sessoes)
                Divisor()
                LazyColumn(
                    Modifier
                        .fillMaxSize()
                        // 60% quando o dado é cache. O valor vem do documento e é
                        // o mesmo que o desktop usa para conteúdo obsoleto.
                        .alpha(if (conexao is ConnectionState.Conectado) 1f else 0.6f),
                ) {
                    items(sessoes, key = { it.id }) { sessao ->
                        SessionRow(sessao, aoTocar = { aoAbrirSessao(sessao.id) })
                        Divisor()
                    }
                }
            }
        }
    }
}

@Composable
private fun Cabecalho(conexao: ConnectionState, modifier: Modifier = Modifier) {
    Column(modifier) {
        Row(
            Modifier
                .fillMaxWidth()
                .height(56.dp)
                .padding(horizontal = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                LumeLogo()
                Text(
                    text = stringResource(R.string.app_name),
                    style = LumeTheme.typography.title,
                    color = LumeTheme.colors.ink,
                )
            }
            PontoDeConexao(conexao)
        }
        Divisor()
    }
}

@Composable
private fun FaixaDeDesconexao(nomeDoDesktop: String, modifier: Modifier = Modifier) {
    Row(
        modifier
            .fillMaxWidth()
            .background(LumeTheme.colors.surfaceRaised)
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        Box(
            Modifier
                .size(8.dp)
                .clip(CircleShape)
                .background(LumeTheme.states.idle.fill),
        )
        Text(
            text = stringResource(R.string.conexao_faixa_offline, nomeDoDesktop),
            style = LumeTheme.typography.label,
            color = LumeTheme.colors.inkMuted,
        )
    }
}

@Composable
private fun ResumoComMascote(conexao: ConnectionState, sessoes: List<AgentSession>) {
    Column(
        Modifier
            .fillMaxWidth()
            .padding(start = 16.dp, end = 16.dp, top = 22.dp, bottom = 18.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        LumeMascot(estadoDoMascote(conexao, sessoes), largura = 96.dp)
        Text(
            text = if (conexao is ConnectionState.Desconectado) {
                stringResource(R.string.conexao_ultimo_estado, conexao.ultimoContato.orEmpty())
            } else {
                resumoDasSessoes(sessoes)
            },
            style = LumeTheme.typography.body,
            color = if (conexao is ConnectionState.Conectado) {
                LumeTheme.colors.ink
            } else {
                LumeTheme.colors.inkMuted
            },
            textAlign = TextAlign.Center,
        )
    }
}

@Composable
private fun EstadoDeErro(nomeDoDesktop: String, aoTentarNovamente: () -> Unit) {
    EstadoVazio(
        titulo = stringResource(R.string.erro_titulo, nomeDoDesktop),
        corpo = stringResource(R.string.erro_corpo),
        marca = {
            // A mesma exclamação do mascote, na cor de falha — é a gramática que o
            // design usa nesta tela, e não um ícone novo.
            GlifoDeAtencao(tamanho = 36.dp)
            Spacer(Modifier.height(6.dp))
        },
        acao = {
            Spacer(Modifier.height(10.dp))
            BotaoContornado(
                texto = stringResource(R.string.erro_tentar_novamente),
                aoTocar = aoTentarNovamente,
                modifier = Modifier.fillMaxWidth(0.6f),
            )
        },
    )
}
