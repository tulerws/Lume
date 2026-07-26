package com.tulerws.lume.mobile.ui.previews

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
import com.tulerws.lume.mobile.data.fake.DadosDoDesign
import com.tulerws.lume.mobile.ui.EstadoDePareamento
import com.tulerws.lume.mobile.data.update.EstadoDaAtualizacao
import com.tulerws.lume.mobile.domain.ConnectionState
import com.tulerws.lume.mobile.domain.PermissionAction
import com.tulerws.lume.mobile.domain.PermissionProfile
import com.tulerws.lume.mobile.domain.AccessMode
import com.tulerws.lume.mobile.domain.acoesRespondiveis
import com.tulerws.lume.mobile.ui.components.BottomBar
import com.tulerws.lume.mobile.ui.components.Destino
import com.tulerws.lume.mobile.ui.components.EstadoDoMascote
import com.tulerws.lume.mobile.ui.components.LumeMascot
import com.tulerws.lume.mobile.ui.components.PermissionBlock
import com.tulerws.lume.mobile.ui.history.HistoryScreen
import com.tulerws.lume.mobile.ui.pair.ManualEntryScreen
import com.tulerws.lume.mobile.ui.pair.PairScreen
import com.tulerws.lume.mobile.ui.session.PermissaoResolvida
import com.tulerws.lume.mobile.ui.session.SessionScreen
import com.tulerws.lume.mobile.ui.sessions.SessionsScreen
import com.tulerws.lume.mobile.ui.settings.SettingsScreen
import com.tulerws.lume.mobile.ui.theme.LumeTheme
import com.tulerws.lume.mobile.ui.theme.ThemeMode

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
 * propósito (`LocalInspectionMode`), a câmera do pareamento, e o comportamento
 * real de conexão — esses só se veem no aparelho.
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
        pareado = true,
        aoParear = {},
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
        pareado = true,
        aoParear = {},
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
        pareado = true,
        aoParear = {},
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
        pareado = true,
        aoParear = {},
        nomeDoDesktop = dados.desktop.nome,
    )
}

@Preview(name = "Sessões · nunca pareado", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun SessoesSemAparelho() = Moldura(ThemeMode.Light) {
    SessionsScreen(
        conexao = ConnectionState.Desconectado(),
        sessoes = emptyList(),
        aoAbrirSessao = {},
        aoTentarNovamente = {},
        // O estado que faltava: nunca houve pareamento. Mascote dormindo, e uma
        // ação em vez de um beco.
        pareado = false,
        aoParear = {},
        nomeDoDesktop = "",
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
        pareado = true,
        aoParear = {},
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
        falhaDaAcao = null,
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
        falhaDaAcao = null,
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
        falhaDaAcao = null,
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
    HistoryScreen(entradas = dados.historico, carregando = false, noTeto = true, falha = null, aoChegarNoFim = {}, aoTentarNovamente = {})
}

@Preview(name = "Histórico · escuro", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun HistoricoEscuro() = Moldura(ThemeMode.Dark) {
    HistoryScreen(entradas = dados.historico, carregando = false, noTeto = true, falha = null, aoChegarNoFim = {}, aoTentarNovamente = {})
}

@Preview(name = "Histórico · vazio", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun HistoricoVazio() = Moldura(ThemeMode.Light) {
    HistoryScreen(entradas = emptyList(), carregando = false, noTeto = false, falha = null, aoChegarNoFim = {}, aoTentarNovamente = {})
}

// ── Ajustes ──────────────────────────────────────────────────────────────────

@Preview(name = "Histórico · sem conexão", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun HistoricoSemConexao() = Moldura(ThemeMode.Light) {
    // O estado que crashava: sem pareamento, o pedido nem chega a sair.
    HistoryScreen(
        entradas = emptyList(),
        carregando = false,
        noTeto = false,
        falha = "desconectado",
        aoChegarNoFim = {},
        aoTentarNovamente = {},
    )
}

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
        atualizacao = EstadoDaAtualizacao.EmDia,
        aoTrocarTema = {},
        aoTrocarAvisoDePermissao = {},
        aoTrocarAvisoDeConclusao = {},
        aoEsquecerDesktop = {},
        aoParear = {},
        aoVerificarAtualizacao = {},
        aoBaixarAtualizacao = {},
        aoInstalarAtualizacao = {},
        aoAutorizarInstalacao = {},
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
        atualizacao = EstadoDaAtualizacao.Disponivel("0.5.4", null),
        aoTrocarTema = {},
        aoTrocarAvisoDePermissao = {},
        aoTrocarAvisoDeConclusao = {},
        aoEsquecerDesktop = {},
        aoParear = {},
        aoVerificarAtualizacao = {},
        aoBaixarAtualizacao = {},
        aoInstalarAtualizacao = {},
        aoAutorizarInstalacao = {},
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
        nomeDoDesktop = null,
        aoLerQr = {},
        aoFechar = {},
        aoDigitarEndereco = {},
        aoConcluir = {},
        aoTentarDeNovo = {},
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

// ── Barra inferior flutuante ─────────────────────────────────────────────────

/**
 * A barra sozinha, com cada destino selecionado.
 *
 * Ela vive no `LumeApp` e não aparece nos previews de tela, então sem isto o
 * componente que mais mudou seria o único sem conferência visual — que foi
 * exatamente como o defeito do inset chegou ao aparelho.
 *
 * O fundo escuro deliberado mostra a pílula recortada contra o conteúdo, que é
 * como ela aparece flutuando sobre uma lista.
 */
@Preview(name = "Barra · claro", widthDp = LARGURA, heightDp = 120)
@Composable
private fun BarraClara() = Moldura(ThemeMode.Light) {
    ColunaDeBarras()
}

@Preview(name = "Barra · escuro", widthDp = LARGURA, heightDp = 120)
@Composable
private fun BarraEscura() = Moldura(ThemeMode.Dark) {
    ColunaDeBarras()
}

@Composable
private fun ColunaDeBarras() {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Destino.entries.forEach { destino ->
            BottomBar(selecionado = destino, aoSelecionar = {})
        }
    }
}

// ── Sessão encerrada ─────────────────────────────────────────────────────────

@Preview(name = "Sessão · ação falhou", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun SessaoAcaoFalhou() = Moldura(ThemeMode.Light) {
    // O estado que era 100% silencioso: responder permissão sem rede.
    SessionScreen(
        sessao = dados.sessoes.first { it.id == "s-claude-lume" },
        permissaoResolvida = null,
        falhaDaAcao = "desconectado",
        prompt = "",
        aoMudarPrompt = {},
        aoEnviarPrompt = {},
        aoResponderPermissao = {},
        aoVoltar = {},
    )
}

@Preview(name = "Sessão · encerrada", widthDp = LARGURA, heightDp = ALTURA)
@Composable
private fun SessaoEncerrada() = Moldura(ThemeMode.Light) {
    SessionScreen(
        sessao = dados.sessoes.first { it.id == "s-claude-android" },
        // O conteúdo continua legível, a faixa diz o que houve, e o campo de
        // prompt fica desabilitado com o motivo escrito.
        encerrada = true,
        permissaoResolvida = null,
        falhaDaAcao = null,
        prompt = "",
        aoMudarPrompt = {},
        aoEnviarPrompt = {},
        aoResponderPermissao = {},
        aoVoltar = {},
    )
}
