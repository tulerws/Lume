package lume.ai.ui.previews

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import lume.ai.data.fake.DadosDoDesign
import lume.ai.data.EstadoDePareamento
import lume.ai.domain.ConnectionState
import lume.ai.domain.PermissionAction
import lume.ai.domain.PermissionProfile
import lume.ai.domain.AccessMode
import lume.ai.domain.acoesRespondiveis
import lume.ai.ui.components.EstadoDoMascote
import lume.ai.ui.components.LumeMascot
import lume.ai.ui.components.PermissionBlock
import lume.ai.ui.history.HistoryScreen
import lume.ai.ui.pair.ManualEntryScreen
import lume.ai.ui.pair.PairScreen
import lume.ai.ui.session.PermissaoResolvida
import lume.ai.ui.session.SessionScreen
import lume.ai.ui.sessions.SessionsScreen
import lume.ai.ui.settings.SettingsScreen
import lume.ai.ui.theme.LumeTheme
import lume.ai.ui.theme.ThemeMode

/**
 * Um `@Preview` por quadro do arquivo de design, com o mesmo nome que ele usa.
 *
 * É assim que este trabalho é conferido: o Android Studio renderiza os quadros
 * lado a lado, na mesma medida do design — 384 × 832 —, e a comparação é visual e
 * direta em vez de de memória.
 *
 * As telas recebem estado por parâmetro e nenhuma delas chama `hiltViewModel`.
 * Isso não foi feito para os previews funcionarem; foi feito porque é o desenho
 * certo, e os previews funcionarem é a consequência que prova que ele está certo.
 *
 * O que os previews **não** cobrem: animação do mascote, que congela aqui de
 * propósito (`LocalInspectionMode`), e os quatro mundos de conexão vistos no
 * aparelho — para isso existe a seção Demonstração em Ajustes.
 */

private const val LARGURA = 384
private const val ALTURA = 832

private val dados = DadosDoDesign()

@Composable
private fun Moldura(modo: ThemeMode, conteudo: @Composable () -> Unit) {
    LumeTheme(mode = modo) {
        Column(Modifier.fillMaxSize().background(LumeTheme.colors.surface)) { conteudo() }
    }
}

// ── Sessões ──────────────────────────────────────────────────────────────────

@Preview(name = "Sessões · claro", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun SessoesClaro() = Moldura(ThemeMode.Light) {
    SessionsScreen(
        conexao = ConnectionState.Conectado,
        sessoes = dados.sessoes,
        aoAbrirSessao = {},
        aoTentarNovamente = {},
        nomeDoDesktop = dados.desktop.nome,
    )
}

@Preview(name = "Sessões · escuro", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun SessoesEscuro() = Moldura(ThemeMode.Dark) {
    SessionsScreen(
        conexao = ConnectionState.Conectado,
        sessoes = dados.sessoes,
        aoAbrirSessao = {},
        aoTentarNovamente = {},
        nomeDoDesktop = dados.desktop.nome,
    )
}

@Preview(name = "Sessões · vazio, mascote desperto", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun SessoesVazio() = Moldura(ThemeMode.Light) {
    SessionsScreen(
        conexao = ConnectionState.Conectado,
        sessoes = emptyList(),
        aoAbrirSessao = {},
        aoTentarNovamente = {},
        nomeDoDesktop = dados.desktop.nome,
    )
}

@Preview(name = "Sessões · sem conexão, cache a 60%", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun SessoesSemConexao() = Moldura(ThemeMode.Dark) {
    SessionsScreen(
        conexao = ConnectionState.Desconectado(ultimoContato = "há 6 min"),
        sessoes = dados.sessoes.take(3),
        aoAbrirSessao = {},
        aoTentarNovamente = {},
        nomeDoDesktop = dados.desktop.nome,
    )
}

@Preview(name = "Sessões · erro de conexão", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun SessoesErro() = Moldura(ThemeMode.Light) {
    SessionsScreen(
        conexao = ConnectionState.Erro("sem rota"),
        sessoes = emptyList(),
        aoAbrirSessao = {},
        aoTentarNovamente = {},
        nomeDoDesktop = dados.desktop.nome,
    )
}

// ── Sessão ───────────────────────────────────────────────────────────────────

@Preview(name = "Sessão · claro, risco alto", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun SessaoRiscoAlto() = Moldura(ThemeMode.Light) {
    SessionScreen(
        sessao = dados.sessoes.first { it.id == "s-claude-lume" },
        permissaoResolvida = null,
        prompt = "",
        aoMudarPrompt = {},
        aoEnviarPrompt = {},
        aoResponderPermissao = {},
        aoVoltar = {},
    )
}

@Preview(name = "Sessão · escuro, permissão respondida", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun SessaoRespondida() = Moldura(ThemeMode.Dark) {
    SessionScreen(
        sessao = dados.sessoes.first { it.id == "s-codex-gateway" },
        permissaoResolvida = PermissaoResolvida(
            acao = PermissionAction.AllowOnce,
            horario = "14:26",
            emOutroDispositivo = true,
        ),
        prompt = "",
        aoMudarPrompt = {},
        aoEnviarPrompt = {},
        aoResponderPermissao = {},
        aoVoltar = {},
    )
}

@Preview(name = "Sessão · sem permissão, prompt livre", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun SessaoSemPermissao() = Moldura(ThemeMode.Light) {
    SessionScreen(
        sessao = dados.sessoes.first { it.id == "s-claude-pipeline" },
        permissaoResolvida = null,
        prompt = "",
        aoMudarPrompt = {},
        aoEnviarPrompt = {},
        aoResponderPermissao = {},
        aoVoltar = {},
    )
}

// ── Bloco de permissão: as variantes que o design não desenha ────────────────

@Preview(name = "Permissão · todas as ações", widthDp = LARGURA)
@Composable
private fun PermissaoCompleta() = Moldura(ThemeMode.Light) {
    val sessao = dados.sessoes.first { it.id == "s-claude-lume" }
    PermissionBlock(
        pedido = sessao.pendingPermission!!,
        acoes = sessao.acoesRespondiveis(),
        caminho = sessao.workingDirectory,
        aoResponder = {},
    )
}

@Preview(name = "Permissão · sem permitir na sessão", widthDp = LARGURA)
@Composable
private fun PermissaoSemSessao() = Moldura(ThemeMode.Light) {
    val sessao = dados.sessoes.first { it.id == "s-claude-lume" }
    PermissionBlock(
        pedido = sessao.pendingPermission!!,
        // O que o servidor mandar é o que aparece. Um bloco que assumisse as três
        // ações mentiria sobre `availableActions`, e é o único jeito de este
        // componente estar errado.
        acoes = listOf(PermissionAction.AllowOnce, PermissionAction.Deny),
        caminho = sessao.workingDirectory,
        aoResponder = {},
    )
}

@Preview(name = "Permissão · somente observação", widthDp = LARGURA)
@Composable
private fun PermissaoSomenteObservacao() = Moldura(ThemeMode.Light) {
    val sessao = dados.sessoes.first { it.id == "s-claude-lume" }
    PermissionBlock(
        pedido = sessao.pendingPermission!!,
        // `canRespondFromLume = false`: nenhum botão, e a razão escrita.
        acoes = sessao.copy(
            permissionProfile = PermissionProfile(
                mode = AccessMode.Custom,
                label = "Gerenciada na origem",
                approvalPolicy = "external",
                canRespondFromLume = false,
                availableActions = listOf(PermissionAction.AllowOnce, PermissionAction.Deny),
            ),
        ).acoesRespondiveis(),
        caminho = sessao.workingDirectory,
        aoResponder = {},
    )
}

// ── Histórico ────────────────────────────────────────────────────────────────

@Preview(name = "Histórico · claro", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun HistoricoClaro() = Moldura(ThemeMode.Light) {
    HistoryScreen(entradas = dados.historico, carregando = false, noTeto = true, aoChegarNoFim = {})
}

@Preview(name = "Histórico · escuro", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun HistoricoEscuro() = Moldura(ThemeMode.Dark) {
    HistoryScreen(entradas = dados.historico, carregando = false, noTeto = true, aoChegarNoFim = {})
}

@Preview(name = "Histórico · vazio", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun HistoricoVazio() = Moldura(ThemeMode.Light) {
    HistoryScreen(entradas = emptyList(), carregando = false, noTeto = false, aoChegarNoFim = {})
}

// ── Ajustes ──────────────────────────────────────────────────────────────────

@Preview(name = "Ajustes · claro", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun AjustesClaro() = Moldura(ThemeMode.Light) {
    SettingsScreen(
        desktop = dados.desktop,
        conexao = ConnectionState.Conectado,
        tema = ThemeMode.Light,
        avisarPermissao = true,
        avisarConclusao = false,
        versao = "0.5.0",
        aoTrocarTema = {},
        aoTrocarAvisoDePermissao = {},
        aoTrocarAvisoDeConclusao = {},
        aoEsquecerDesktop = {},
    )
}

@Preview(name = "Ajustes · escuro", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun AjustesEscuro() = Moldura(ThemeMode.Dark) {
    SettingsScreen(
        desktop = dados.desktop,
        conexao = ConnectionState.Conectado,
        tema = ThemeMode.Dark,
        avisarPermissao = true,
        avisarConclusao = false,
        versao = "0.5.0",
        aoTrocarTema = {},
        aoTrocarAvisoDePermissao = {},
        aoTrocarAvisoDeConclusao = {},
        aoEsquecerDesktop = {},
    )
}

// ── Pareamento ───────────────────────────────────────────────────────────────

@Preview(name = "Pareamento", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun Pareamento() = Moldura(ThemeMode.Dark) {
    PairScreen(
        // Sem câmera no preview: `LocalInspectionMode` não impede o CameraX de
        // tentar, então o que se confere aqui é o gradiente, o enquadramento e a
        // hierarquia de texto — que é o que o design desenha.
        estado = EstadoDePareamento.Ocioso,
        falhaDeLeitura = null,
        aoLerQr = {},
        aoFechar = {},
        aoDigitarEndereco = {},
        aoConcluir = {},
    )
}

@Preview(name = "Pareamento · entrada manual", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun EntradaManual() = Moldura(ThemeMode.Light) {
    ManualEntryScreen(aoVoltar = {}, aoConectar = { _, _ -> })
}

// ── Mascote ──────────────────────────────────────────────────────────────────

/**
 * Os sete estados lado a lado.
 *
 * O design desenha três. Este preview é onde se confere que os outros quatro
 * pertencem à mesma família — e não a um desenho novo que alguém inventou.
 */
@Preview(name = "Mascote · sete estados, claro", widthDp = LARGURA)
@Composable
private fun MascoteClaro() = Moldura(ThemeMode.Light) {
    GradeDeMascotes()
}

@Preview(name = "Mascote · sete estados, escuro", widthDp = LARGURA)
@Composable
private fun MascoteEscuro() = Moldura(ThemeMode.Dark) {
    GradeDeMascotes()
}

@Composable
private fun GradeDeMascotes() {
    Column(
        Modifier.padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        EstadoDoMascote.entries.chunked(3).forEach { linha ->
            Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                linha.forEach { estado ->
                    LumeMascot(estado, largura = 72.dp)
                }
            }
        }
    }
}
