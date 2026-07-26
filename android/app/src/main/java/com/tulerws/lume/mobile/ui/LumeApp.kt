package com.tulerws.lume.mobile.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import com.tulerws.lume.mobile.ui.components.IconeVoltar
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import com.tulerws.lume.mobile.BuildConfig
import com.tulerws.lume.mobile.ui.components.BottomBar
import com.tulerws.lume.mobile.ui.components.Destino
import com.tulerws.lume.mobile.ui.history.HistoryScreen
import com.tulerws.lume.mobile.ui.pair.ManualEntryScreen
import com.tulerws.lume.mobile.ui.pair.PairScreen
import com.tulerws.lume.mobile.ui.session.SessionScreen
import com.tulerws.lume.mobile.ui.sessions.SessionsScreen
import com.tulerws.lume.mobile.ui.settings.SettingsScreen
import com.tulerws.lume.mobile.ui.theme.LumeTheme

/**
 * Rotas.
 *
 * Constantes em vez de literais espalhados: rota escrita errada não é erro de
 * compilação, é tela em branco em tempo de execução.
 */
private object Rotas {
    const val SESSOES = "sessoes"
    const val HISTORICO = "historico"
    const val AJUSTES = "ajustes"
    const val SESSAO = "sessao/{id}"
    const val PAREAR = "parear"
    const val MANUAL = "manual"

    fun sessao(id: String) = "sessao/$id"
}

/**
 * A árvore inteira do aplicativo.
 *
 * A barra inferior fica **fora** do `NavHost` e aparece só nos três destinos. Pôr
 * a barra dentro de cada tela repetiria o componente quatro vezes e faria a
 * transição entre abas animar a barra junto, que é o defeito visual mais comum
 * dessa montagem.
 *
 * Pareamento, entrada manual e detalhe de sessão não têm barra: os dois primeiros
 * são fluxo de abertura, o terceiro é empilhamento.
 */
@Composable
fun LumeApp() {
    val app: AppViewModel = hiltViewModel()
    val tema by app.tema.collectAsStateWithLifecycle()
    val desktop by app.desktop.collectAsStateWithLifecycle()

    LumeTheme(mode = tema) {
        val navegacao = rememberNavController()
        val entradaAtual by navegacao.currentBackStackEntryAsState()
        val rotaAtual = entradaAtual?.destination?.route

        val destinoAtual = when (rotaAtual) {
            Rotas.SESSOES -> Destino.Sessoes
            Rotas.HISTORICO -> Destino.Historico
            Rotas.AJUSTES -> Destino.Ajustes
            else -> null
        }

        // `Box`, e não `Column`: a barra flutua **sobre** o conteúdo. Empilhada
        // numa coluna ela roubaria altura da tela, que é o oposto de flutuar.
        Box(
            Modifier
                .fillMaxSize()
                .background(LumeTheme.colors.surface),
        ) {
            NavHost(
                    navController = navegacao,
                    // **Sempre** Sessões, mesmo sem aparelho pareado.
                    //
                    // Antes o pareamento era o destino inicial quando não havia
                    // credencial, e o X daquela tela chamava `popBackStack()`:
                    // desempilhava a única entrada e deixava o `NavHost` sem nada
                    // para mostrar. O aplicativo congelava até ser reiniciado.
                    //
                    // Com Sessões na raiz, o pareamento vira empilhamento — a
                    // partir de Ajustes ou do estado vazio — e o X sempre tem para
                    // onde voltar. O congelamento deixa de ser possível por
                    // construção, e não por conserto.
                    startDestination = Rotas.SESSOES,
                ) {
                    composable(Rotas.SESSOES) {
                        val vm: SessionsViewModel = hiltViewModel()
                        val conexao by vm.conexao.collectAsStateWithLifecycle()
                        val sessoes by vm.sessoes.collectAsStateWithLifecycle()
                        SessionsScreen(
                            conexao = conexao,
                            sessoes = sessoes,
                            aoAbrirSessao = { navegacao.navigate(Rotas.sessao(it)) },
                            aoTentarNovamente = vm::tentarNovamente,
                            pareado = desktop != null,
                            aoParear = { navegacao.navigate(Rotas.PAREAR) },
                            nomeDoDesktop = desktop?.nome.orEmpty(),
                        )
                    }

                    composable(Rotas.HISTORICO) {
                        val vm: HistoryViewModel = hiltViewModel()
                        val entradas by vm.entradas.collectAsStateWithLifecycle()
                        val carregando by vm.carregando.collectAsStateWithLifecycle()
                        val noTeto by vm.noTeto.collectAsStateWithLifecycle()
                        val falha by vm.falha.collectAsStateWithLifecycle()
                        HistoryScreen(
                            entradas = entradas,
                            carregando = carregando,
                            noTeto = noTeto,
                            falha = falha,
                            aoChegarNoFim = vm::carregarMais,
                            aoTentarNovamente = vm::tentarNovamente,
                        )
                    }

                    composable(Rotas.AJUSTES) {
                        val vm: SettingsViewModel = hiltViewModel()
                        val desk by vm.desktop.collectAsStateWithLifecycle()
                        val conexao by vm.conexao.collectAsStateWithLifecycle()
                        val temaAtual by vm.tema.collectAsStateWithLifecycle()
                        val permissao by vm.avisarPermissao.collectAsStateWithLifecycle()
                        val conclusao by vm.avisarConclusao.collectAsStateWithLifecycle()
                        val atualizacao by vm.atualizacao.collectAsStateWithLifecycle()
                        SettingsScreen(
                            desktop = desk,
                            conexao = conexao,
                            tema = temaAtual,
                            avisarPermissao = permissao,
                            avisarConclusao = conclusao,
                            versao = BuildConfig.VERSION_NAME,
                            atualizacao = atualizacao,
                            aoTrocarTema = vm::definirTema,
                            aoTrocarAvisoDePermissao = vm::definirAvisoDePermissao,
                            aoTrocarAvisoDeConclusao = vm::definirAvisoDeConclusao,
                            aoEsquecerDesktop = vm::esquecerDesktop,
                            aoParear = { navegacao.navigate(Rotas.PAREAR) },
                            aoVerificarAtualizacao = vm::verificarAtualizacao,
                            aoBaixarAtualizacao = vm::baixarAtualizacao,
                            aoInstalarAtualizacao = vm::instalarAtualizacao,
                            aoAutorizarInstalacao = vm::autorizarInstalacao,
                        )
                    }

                    composable(Rotas.SESSAO) {
                        val vm: SessionViewModel = hiltViewModel()
                        val estadoDaSessao by vm.sessao.collectAsStateWithLifecycle()
                        val prompt by vm.prompt.collectAsStateWithLifecycle()
                        val resolvida by vm.permissaoResolvida.collectAsStateWithLifecycle()
                        val falhaDaAcao by vm.falhaDaAcao.collectAsStateWithLifecycle()
                        // Nenhum `popBackStack()` automático. A tela some por baixo
                        // de quem está lendo, e foi essa mecânica que fez tocar
                        // numa sessão não abrir nada.
                        when (val atual = estadoDaSessao) {
                            is EstadoDaSessao.Carregando ->
                                CarregandoSessao(aoVoltar = { navegacao.popBackStack() })

                            is EstadoDaSessao.Ativa -> SessionScreen(
                                sessao = atual.sessao,
                                encerrada = false,
                                permissaoResolvida = resolvida,
                                falhaDaAcao = falhaDaAcao,
                                prompt = prompt,
                                aoMudarPrompt = vm::mudarPrompt,
                                aoEnviarPrompt = vm::enviarPrompt,
                                aoResponderPermissao = vm::responderPermissao,
                                aoVoltar = { navegacao.popBackStack() },
                            )

                            is EstadoDaSessao.Encerrada -> SessionScreen(
                                sessao = atual.ultimaConhecida,
                                encerrada = true,
                                permissaoResolvida = resolvida,
                                falhaDaAcao = falhaDaAcao,
                                prompt = prompt,
                                aoMudarPrompt = vm::mudarPrompt,
                                aoEnviarPrompt = vm::enviarPrompt,
                                aoResponderPermissao = vm::responderPermissao,
                                aoVoltar = { navegacao.popBackStack() },
                            )
                        }
                    }

                    composable(Rotas.PAREAR) {
                        val vm: PairViewModel = hiltViewModel()
                        val estadoDoPareamento by vm.estado.collectAsStateWithLifecycle()
                        val falha by vm.falhaDeLeitura.collectAsStateWithLifecycle()
                        val nomeLido by vm.nomeDoDesktop.collectAsStateWithLifecycle()
                        PairScreen(
                            estado = estadoDoPareamento,
                            falhaDeLeitura = falha,
                            nomeDoDesktop = nomeLido,
                            aoLerQr = vm::aoLerQr,
                            // Guarda de cinto e suspensório: com Sessões na raiz a
                            // pilha nunca fica vazia, mas um X que não faz nada é
                            // pior do que um X que volta ao começo.
                            aoFechar = {
                                if (!navegacao.popBackStack()) {
                                    navegacao.navigate(Rotas.SESSOES) {
                                        popUpTo(Rotas.SESSOES) { inclusive = true }
                                    }
                                }
                            },
                            aoDigitarEndereco = { navegacao.navigate(Rotas.MANUAL) },
                            aoTentarDeNovo = vm::tentarDeNovo,
                            // Pareou: Sessões passa a ser o começo, e a tela de
                            // pareamento sai da pilha. Voltar dali não deve
                            // devolver a câmera a quem já pareou.
                            aoConcluir = {
                                // `launchSingleTop`: sem ele a pilha virava
                                // [SESSOES, SESSOES] e o gesto de voltar parecia
                                // falhar na primeira vez.
                                navegacao.navigate(Rotas.SESSOES) {
                                    popUpTo(Rotas.PAREAR) { inclusive = true }
                                    launchSingleTop = true
                                }
                            },
                        )
                    }

                    composable(Rotas.MANUAL) {
                        ManualEntryScreen(
                            aoVoltar = { navegacao.popBackStack() },
                            // Sem cliente de rede não há o que conectar. A tela
                            // valida o formato e volta; ligar isto ao
                            // `PairingRepository` é uma linha, quando ele souber
                            // conectar.
                            aoConectar = { _, _ -> navegacao.popBackStack() },
                        )
                    }
                }

            if (destinoAtual != null) {
                BottomBar(
                    selecionado = destinoAtual,
                    aoSelecionar = { destino -> irPara(navegacao, destino) },
                    modifier = Modifier.align(Alignment.BottomCenter),
                )
            }
        }
    }
}

/**
 * Troca de aba.
 *
 * `launchSingleTop` mais `popUpTo` no início do grafo: sem eles, alternar entre
 * abas empilha uma entrada a cada toque, e o gesto de voltar passa a percorrer o
 * histórico de abas em vez de sair do aplicativo. É o defeito mais comum de
 * barra inferior com Navigation Compose.
 *
 * `saveState` e `restoreState` preservam a rolagem de cada aba, que é o que faz a
 * volta ao Histórico não recomeçar do topo.
 */
private fun irPara(navegacao: NavHostController, destino: Destino) {
    val rota = when (destino) {
        Destino.Sessoes -> Rotas.SESSOES
        Destino.Historico -> Rotas.HISTORICO
        Destino.Ajustes -> Rotas.AJUSTES
    }
    navegacao.navigate(rota) {
        popUpTo(navegacao.graph.startDestinationId) { saveState = true }
        launchSingleTop = true
        restoreState = true
    }
}

/**
 * Enquanto o primeiro snapshot não chega.
 *
 * Sem indicador girando — ele pisca mais do que informa quando a conexão já está
 * viva. **Mas com botão de voltar.** A versão anterior era um `Box` vazio, e
 * quando o estado ficava preso aqui a tela virava beco: nem cabeçalho, nem saída,
 * só o gesto do sistema. Uma tela sem saída está errada mesmo quando o estado que
 * a produz está certo.
 */
@Composable
private fun CarregandoSessao(aoVoltar: () -> Unit) {
    Column(
        Modifier
            .fillMaxSize()
            .background(LumeTheme.colors.surface)
            .statusBarsPadding(),
    ) {
        Box(
            Modifier
                .padding(horizontal = 8.dp)
                .size(48.dp)
                .clickable(onClick = aoVoltar),
            contentAlignment = Alignment.Center,
        ) {
            IconeVoltar(LumeTheme.colors.ink)
        }
    }
}
