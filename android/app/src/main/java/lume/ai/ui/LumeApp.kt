package lume.ai.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import lume.ai.BuildConfig
import lume.ai.ui.components.BottomBar
import lume.ai.ui.components.Destino
import lume.ai.ui.history.HistoryScreen
import lume.ai.ui.pair.ManualEntryScreen
import lume.ai.ui.pair.PairScreen
import lume.ai.ui.session.SessionScreen
import lume.ai.ui.sessions.SessionsScreen
import lume.ai.ui.settings.SettingsScreen
import lume.ai.ui.theme.LumeTheme

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

        // Lido **uma vez**. `startDestination` define o grafo, e trocá-lo depois
        // faz o `NavHost` reconstruir o grafo com a pilha atual em cima — que é
        // como se chega numa pilha que aponta para rota que não existe mais.
        // Esquecer o aparelho navega explicitamente; não depende deste valor.
        val destinoInicial = remember { if (desktop == null) Rotas.PAREAR else Rotas.SESSOES }

        val destinoAtual = when (rotaAtual) {
            Rotas.SESSOES -> Destino.Sessoes
            Rotas.HISTORICO -> Destino.Historico
            Rotas.AJUSTES -> Destino.Ajustes
            else -> null
        }

        Column(
            Modifier
                .fillMaxSize()
                .background(LumeTheme.colors.surface),
        ) {
            Box(Modifier.weight(1f)) {
                NavHost(
                    navController = navegacao,
                    // Sem aparelho pareado não há o que mostrar, e a tela de
                    // pareamento é a abertura. Com aparelho, entra direto em
                    // Sessões — que é a razão de o aplicativo ser aberto.
                    startDestination = destinoInicial,
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
                            nomeDoDesktop = desktop?.nome.orEmpty(),
                        )
                    }

                    composable(Rotas.HISTORICO) {
                        val vm: HistoryViewModel = hiltViewModel()
                        val entradas by vm.entradas.collectAsStateWithLifecycle()
                        val carregando by vm.carregando.collectAsStateWithLifecycle()
                        val noTeto by vm.noTeto.collectAsStateWithLifecycle()
                        HistoryScreen(
                            entradas = entradas,
                            carregando = carregando,
                            noTeto = noTeto,
                            aoChegarNoFim = vm::carregarMais,
                        )
                    }

                    composable(Rotas.AJUSTES) {
                        val vm: SettingsViewModel = hiltViewModel()
                        val desk by vm.desktop.collectAsStateWithLifecycle()
                        val conexao by vm.conexao.collectAsStateWithLifecycle()
                        val temaAtual by vm.tema.collectAsStateWithLifecycle()
                        val permissao by vm.avisarPermissao.collectAsStateWithLifecycle()
                        val conclusao by vm.avisarConclusao.collectAsStateWithLifecycle()
                        SettingsScreen(
                            desktop = desk,
                            conexao = conexao,
                            tema = temaAtual,
                            avisarPermissao = permissao,
                            avisarConclusao = conclusao,
                            versao = BuildConfig.VERSION_NAME,
                            aoTrocarTema = vm::definirTema,
                            aoTrocarAvisoDePermissao = vm::definirAvisoDePermissao,
                            aoTrocarAvisoDeConclusao = vm::definirAvisoDeConclusao,
                            aoEsquecerDesktop = {
                                vm.esquecerDesktop()
                                irParaPareamento(navegacao)
                            },
                        )
                    }

                    composable(Rotas.SESSAO) {
                        val vm: SessionViewModel = hiltViewModel()
                        val sessao by vm.sessao.collectAsStateWithLifecycle()
                        val prompt by vm.prompt.collectAsStateWithLifecycle()
                        val resolvida by vm.permissaoResolvida.collectAsStateWithLifecycle()
                        // `null` significa que a sessão sumiu do snapshot enquanto
                        // a tela estava aberta — situação normal, não erro. Voltar
                        // é o comportamento certo; mostrar uma tela vazia com o
                        // nome de uma sessão que não existe mais, não.
                        val atual = sessao
                        if (atual == null) {
                            VoltarQuandoSessaoSome(navegacao)
                        } else {
                            SessionScreen(
                                sessao = atual,
                                permissaoResolvida = resolvida,
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
                        PairScreen(
                            estado = estadoDoPareamento,
                            falhaDeLeitura = falha,
                            aoLerQr = vm::aoLerQr,
                            aoFechar = { navegacao.popBackStack() },
                            aoDigitarEndereco = { navegacao.navigate(Rotas.MANUAL) },
                            // Pareou: Sessões passa a ser o começo, e a tela de
                            // pareamento sai da pilha. Voltar dali não deve
                            // devolver a câmera a quem já pareou.
                            aoConcluir = {
                                navegacao.navigate(Rotas.SESSOES) {
                                    popUpTo(Rotas.PAREAR) { inclusive = true }
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
            }

            if (destinoAtual != null) {
                BottomBar(
                    selecionado = destinoAtual,
                    aoSelecionar = { destino -> irPara(navegacao, destino) },
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

/** Esquecer o aparelho leva de volta ao pareamento, sem deixar nada na pilha. */
private fun irParaPareamento(navegacao: NavHostController) {
    navegacao.navigate(Rotas.PAREAR) {
        popUpTo(navegacao.graph.id) { inclusive = true }
    }
}

@Composable
private fun VoltarQuandoSessaoSome(navegacao: NavHostController) {
    androidx.compose.runtime.LaunchedEffect(Unit) { navegacao.popBackStack() }
}
