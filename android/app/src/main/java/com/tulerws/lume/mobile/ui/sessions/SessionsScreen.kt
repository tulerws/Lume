package com.tulerws.lume.mobile.ui.sessions

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
import com.tulerws.lume.mobile.R
import com.tulerws.lume.mobile.domain.AgentSession
import com.tulerws.lume.mobile.domain.ConnectionState
import com.tulerws.lume.mobile.ui.components.BotaoContornado
import com.tulerws.lume.mobile.ui.components.Divisor
import com.tulerws.lume.mobile.ui.components.espacoParaBarra
import com.tulerws.lume.mobile.ui.components.EstadoDoMascote
import com.tulerws.lume.mobile.ui.components.EstadoVazio
import com.tulerws.lume.mobile.ui.components.GlifoDeAtencao
import com.tulerws.lume.mobile.ui.components.LumeLogo
import com.tulerws.lume.mobile.ui.components.LumeMascot
import com.tulerws.lume.mobile.ui.components.PontoDeConexao
import com.tulerws.lume.mobile.ui.components.SessionRow
import com.tulerws.lume.mobile.ui.estadoDoMascote
import com.tulerws.lume.mobile.ui.resumoDasSessoes
import com.tulerws.lume.mobile.ui.theme.LumeTheme

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
    /**
     * Existe credencial guardada. **Não** é o mesmo que estar conectado agora.
     *
     * Pareado e sem rede é situação de espera; nunca pareado não tem a que se
     * reconectar, e a tela precisa oferecer a única ação que resolve.
     */
    pareado: Boolean,
    aoParear: () -> Unit,
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
        // A faixa promete "último estado conhecido". Sem nunca ter pareado não há
        // estado conhecido nenhum, e prometê-lo seria mentira.
        if (pareado && conexao is ConnectionState.Desconectado) {
            FaixaDeDesconexao(nomeDoDesktop, Modifier.statusBarsPadding())
            Cabecalho(conexao)
        } else {
            Cabecalho(conexao, Modifier.statusBarsPadding())
        }

        when {
            !pareado -> EstadoSemAparelho(aoParear)

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
                    contentPadding = espacoParaBarra(),
                    modifier = Modifier
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
                // `null` significa que nunca houve estado conhecido, e o
                // documento diz que a tela **não deve prometer** um. Antes o
                // `.orEmpty()` imprimia "Último estado conhecido · " com o ponto
                // solto, em toda desconexão — porque nada nunca preenche o campo.
                conexao.ultimoContato
                    ?.let { stringResource(R.string.conexao_ultimo_estado, it) }
                    ?: stringResource(R.string.conexao_sem_estado_conhecido)
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

/**
 * Nunca houve pareamento.
 *
 * O mascote dorme, como em qualquer situação sem canal — é o mesmo fato. O que
 * muda é o corpo: aqui existe uma ação, e ela é a única que resolve.
 */
@Composable
private fun EstadoSemAparelho(aoParear: () -> Unit) {
    EstadoVazio(
        titulo = stringResource(R.string.sem_aparelho_titulo),
        corpo = stringResource(R.string.sem_aparelho_corpo),
        marca = {
            LumeMascot(EstadoDoMascote.Dormindo, largura = 96.dp)
            Spacer(Modifier.height(8.dp))
        },
        acao = {
            Spacer(Modifier.height(10.dp))
            BotaoContornado(
                texto = stringResource(R.string.sem_aparelho_acao),
                aoTocar = aoParear,
                modifier = Modifier.fillMaxWidth(0.7f),
            )
        },
    )
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
