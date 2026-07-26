package com.tulerws.lume.mobile.ui.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.tulerws.lume.mobile.R
import com.tulerws.lume.mobile.data.update.EstadoDaAtualizacao
import com.tulerws.lume.mobile.domain.ConnectionState
import com.tulerws.lume.mobile.domain.PairedDesktop
import com.tulerws.lume.mobile.ui.components.Cartao
import com.tulerws.lume.mobile.ui.components.Divisor
import com.tulerws.lume.mobile.ui.components.espacoParaBarra
import com.tulerws.lume.mobile.ui.components.PontoDeConexao
import com.tulerws.lume.mobile.ui.components.RotuloDeSecao
import com.tulerws.lume.mobile.ui.theme.LumeTheme
import com.tulerws.lume.mobile.ui.theme.ThemeMode

/**
 * Tela de Ajustes.
 *
 * Quatro seções no design — Aparelho, Tema, Avisos e Sobre — e **nenhum seletor de
 * idioma**. Essa ausência é decisão, não esquecimento: sem seletor, quem escolhe o
 * idioma é o sistema, que é o mecanismo do Android e o que faz o
 * `values-pt-rBR/strings.xml` funcionar sozinho.
 *
 * **Não há seção de demonstração.** Os quatro estados de conexão do design são
 * vistos nos `@Preview`; no aparelho, o que aparece é o estado real da conexão. Um
 * comutador que fabricasse "Conectado" contradiria o indicador permanente que esta
 * interface existe para manter honesto.
 *
 * A atualização mora dentro de *Sobre*, junto da versão instalada, e não em seção
 * própria: as duas informações respondem à mesma pergunta, e separá-las faria a
 * pessoa ler a versão num lugar e descobrir noutro que ela está velha.
 */
@Composable
fun SettingsScreen(
    desktop: PairedDesktop?,
    conexao: ConnectionState,
    tema: ThemeMode,
    avisarPermissao: Boolean,
    avisarConclusao: Boolean,
    versao: String,
    atualizacao: EstadoDaAtualizacao,
    aoTrocarTema: (ThemeMode) -> Unit,
    aoTrocarAvisoDePermissao: (Boolean) -> Unit,
    aoTrocarAvisoDeConclusao: (Boolean) -> Unit,
    aoEsquecerDesktop: () -> Unit,
    aoParear: () -> Unit,
    aoVerificarAtualizacao: () -> Unit,
    aoBaixarAtualizacao: () -> Unit,
    aoInstalarAtualizacao: () -> Unit,
    aoAutorizarInstalacao: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var confirmandoEsquecer by remember { mutableStateOf(false) }

    Column(
        modifier
            .fillMaxSize()
            .background(LumeTheme.colors.surface),
    ) {
        Column(Modifier.statusBarsPadding()) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .height(56.dp)
                    .padding(horizontal = 16.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = stringResource(R.string.ajustes_titulo),
                    style = LumeTheme.typography.title,
                    color = LumeTheme.colors.ink,
                )
            }
            Divisor()
        }

        Column(
            Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 20.dp)
                .padding(espacoParaBarra()),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            if (desktop == null) {
                // A porta de volta ao scanner. Sem ela, um aplicativo despareado
                // não teria caminho nenhum para parear — que foi o defeito visto
                // no aparelho, onde o X da tela de leitura levava a lugar nenhum.
                Secao(stringResource(R.string.ajustes_aparelho)) {
                    Cartao {
                        Box(
                            Modifier
                                .fillMaxWidth()
                                .height(56.dp)
                                .clickable(onClick = aoParear)
                                .padding(horizontal = 16.dp),
                            contentAlignment = Alignment.CenterStart,
                        ) {
                            Text(
                                text = stringResource(R.string.sem_aparelho_acao),
                                style = LumeTheme.typography.body,
                                color = LumeTheme.colors.accent,
                            )
                        }
                    }
                }
            } else {
                Secao(stringResource(R.string.ajustes_aparelho)) {
                    Cartao {
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .defaultMinSize(minHeight = 56.dp)
                                .padding(horizontal = 16.dp, vertical = 12.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.SpaceBetween,
                        ) {
                            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                                Text(
                                    text = desktop.nome,
                                    style = LumeTheme.typography.body,
                                    color = LumeTheme.colors.ink,
                                )
                                Text(
                                    text = stringResource(
                                        R.string.ajustes_aparelho_detalhe,
                                        desktop.pareadoEm,
                                    ),
                                    style = LumeTheme.typography.label,
                                    color = LumeTheme.colors.inkMuted,
                                )
                            }
                            PontoDeConexao(conexao)
                        }
                        Divisor()
                        Box(
                            Modifier
                                .fillMaxWidth()
                                .height(56.dp)
                                .clickable { confirmandoEsquecer = true }
                                .padding(horizontal = 16.dp),
                            contentAlignment = Alignment.CenterStart,
                        ) {
                            Text(
                                text = stringResource(R.string.ajustes_esquecer),
                                style = LumeTheme.typography.body,
                                // Cor de falha: a ação apaga cache e credencial e
                                // não tem volta. O tratamento precisa dizer isso
                                // antes do diálogo.
                                color = LumeTheme.states.failed.on,
                            )
                        }
                    }
                }
            }

            Secao(stringResource(R.string.ajustes_tema)) {
                SeletorEmLinha(
                    opcoes = listOf(
                        ThemeMode.Light to stringResource(R.string.tema_claro),
                        ThemeMode.Dark to stringResource(R.string.tema_escuro),
                        ThemeMode.System to stringResource(R.string.tema_sistema),
                    ),
                    selecionado = tema,
                    aoSelecionar = aoTrocarTema,
                )
            }

            Secao(stringResource(R.string.ajustes_avisos)) {
                Cartao {
                    LinhaComInterruptor(
                        rotulo = stringResource(R.string.avisos_permissao),
                        ligado = avisarPermissao,
                        aoTrocar = aoTrocarAvisoDePermissao,
                    )
                    Divisor()
                    LinhaComInterruptor(
                        rotulo = stringResource(R.string.avisos_concluida),
                        ligado = avisarConclusao,
                        aoTrocar = aoTrocarAvisoDeConclusao,
                    )
                }
            }

            Secao(stringResource(R.string.ajustes_sobre)) {
                Cartao {
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .defaultMinSize(minHeight = 56.dp)
                            .padding(horizontal = 16.dp, vertical = 12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.SpaceBetween,
                    ) {
                        Text(
                            text = stringResource(R.string.app_name),
                            style = LumeTheme.typography.body,
                            color = LumeTheme.colors.ink,
                        )
                        Text(
                            text = stringResource(R.string.sobre_versao, versao),
                            style = LumeTheme.typography.label,
                            color = LumeTheme.colors.inkMuted,
                        )
                    }
                    Divisor()
                    LinhaDeAtualizacao(
                        estado = atualizacao,
                        aoVerificar = aoVerificarAtualizacao,
                        aoBaixar = aoBaixarAtualizacao,
                        aoInstalar = aoInstalarAtualizacao,
                        aoAutorizar = aoAutorizarInstalacao,
                    )
                }
            }

        }
    }

    if (confirmandoEsquecer && desktop != null) {
        AlertDialog(
            onDismissRequest = { confirmandoEsquecer = false },
            title = {
                Text(
                    stringResource(R.string.ajustes_esquecer_titulo, desktop.nome),
                    style = LumeTheme.typography.title,
                )
            },
            text = {
                Text(
                    stringResource(R.string.ajustes_esquecer_corpo),
                    style = LumeTheme.typography.body,
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    confirmandoEsquecer = false
                    aoEsquecerDesktop()
                }) {
                    Text(
                        stringResource(R.string.ajustes_esquecer_confirmar),
                        color = LumeTheme.states.failed.on,
                        style = LumeTheme.typography.button,
                    )
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmandoEsquecer = false }) {
                    Text(
                        stringResource(R.string.acao_cancelar),
                        color = LumeTheme.colors.inkMuted,
                        style = LumeTheme.typography.button,
                    )
                }
            },
            containerColor = LumeTheme.colors.surfaceRaised,
            titleContentColor = LumeTheme.colors.ink,
            textContentColor = LumeTheme.colors.inkMuted,
            shape = LumeTheme.shapes.card,
        )
    }
}

/**
 * A linha de atualização, dentro de *Sobre*.
 *
 * Cada estado carrega **um** gesto, e nunca mais de um. A tentação seria mostrar
 * "Baixar" e "Verificar de novo" ao mesmo tempo; isso obrigaria a pessoa a escolher
 * entre dois botões para uma decisão que só tem um caminho sensato.
 *
 * [EstadoDaAtualizacao.Ocioso] e [EstadoDaAtualizacao.EmDia] mostram o mesmo gesto
 * com textos diferentes: antes da primeira checagem não se pode afirmar que está em
 * dia, e afirmar assim mesmo seria mentir com aparência de tranquilidade.
 */
@Composable
private fun LinhaDeAtualizacao(
    estado: EstadoDaAtualizacao,
    aoVerificar: () -> Unit,
    aoBaixar: () -> Unit,
    aoInstalar: () -> Unit,
    aoAutorizar: () -> Unit,
) {
    val cores = LumeTheme.colors

    val descricao = when (estado) {
        is EstadoDaAtualizacao.Ocioso -> stringResource(R.string.atualizacao_ociosa)
        is EstadoDaAtualizacao.Verificando -> stringResource(R.string.atualizacao_verificando)
        is EstadoDaAtualizacao.EmDia -> stringResource(R.string.atualizacao_em_dia)
        is EstadoDaAtualizacao.Disponivel ->
            stringResource(R.string.atualizacao_disponivel, estado.versao)
        is EstadoDaAtualizacao.Baixando -> estado.fracao
            ?.let { stringResource(R.string.atualizacao_baixando_pct, (it * 100).toInt()) }
            ?: stringResource(R.string.atualizacao_baixando)
        is EstadoDaAtualizacao.Pronta ->
            stringResource(R.string.atualizacao_pronta, estado.versao)
        is EstadoDaAtualizacao.PrecisaAutorizar -> stringResource(R.string.atualizacao_autorizar)
        is EstadoDaAtualizacao.Falhou -> when (estado.codigo) {
            "rede" -> stringResource(R.string.atualizacao_erro_rede)
            "sha_divergente" -> stringResource(R.string.atualizacao_erro_sha)
            "assinatura_divergente" -> stringResource(R.string.atualizacao_erro_assinatura)
            else -> stringResource(R.string.atualizacao_erro_desconhecido)
        }
    }

    // Falha de conferência não é contratempo de rede: significa que o arquivo que
    // chegou não é o que o manifesto descreve. Merece a cor de falha, e não o
    // cinza de texto acessório.
    val grave = estado is EstadoDaAtualizacao.Falhou &&
        estado.codigo in setOf("sha_divergente", "assinatura_divergente")

    val acao: Pair<String, () -> Unit>? = when (estado) {
        is EstadoDaAtualizacao.Verificando, is EstadoDaAtualizacao.Baixando -> null
        is EstadoDaAtualizacao.Disponivel -> stringResource(R.string.atualizacao_baixar) to aoBaixar
        is EstadoDaAtualizacao.Pronta -> stringResource(R.string.atualizacao_instalar) to aoInstalar
        is EstadoDaAtualizacao.PrecisaAutorizar ->
            stringResource(R.string.atualizacao_abrir_ajustes) to aoAutorizar
        else -> stringResource(R.string.atualizacao_verificar) to aoVerificar
    }

    Row(
        Modifier
            .fillMaxWidth()
            .defaultMinSize(minHeight = 56.dp)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(
            text = descricao,
            style = LumeTheme.typography.label,
            color = if (grave) LumeTheme.states.failed.on else cores.inkMuted,
            modifier = Modifier.weight(1f, fill = false),
        )
        if (acao != null) {
            Text(
                text = acao.first,
                style = LumeTheme.typography.label,
                color = cores.accent,
                modifier = Modifier
                    .padding(start = 12.dp)
                    .clickable(onClick = acao.second)
                    .semantics { role = Role.Button },
            )
        }
    }
}

@Composable
private fun Secao(titulo: String, conteudo: @Composable () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        RotuloDeSecao(titulo)
        conteudo()
    }
}

/**
  * Seletor em linha: preenchido no selecionado, contornado no resto.
 *
 * Não é `SegmentedButton` do Material — aquele desenha um agrupamento com cantos
 * externos arredondados e divisórias internas, e o design pede três botões
 * independentes de 10dp separados por 8dp.
 */
@Composable
private fun <T> SeletorEmLinha(
    opcoes: List<Pair<T, String>>,
    selecionado: T,
    aoSelecionar: (T) -> Unit,
) {
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        opcoes.forEach { (valor, rotulo) ->
            val ativo = valor == selecionado
            Box(
                Modifier
                    .weight(1f)
                    .height(48.dp)
                    .clip(LumeTheme.shapes.field)
                    .background(if (ativo) LumeTheme.colors.accent else androidx.compose.ui.graphics.Color.Transparent)
                    .then(
                        if (ativo) {
                            Modifier
                        } else {
                            Modifier.border(1.dp, LumeTheme.colors.line, LumeTheme.shapes.field)
                        },
                    )
                    .clickable { aoSelecionar(valor) },
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = rotulo,
                    style = LumeTheme.typography.button.copy(fontSize = LumeTheme.typography.label.fontSize),
                    color = if (ativo) LumeTheme.colors.surface else LumeTheme.colors.ink,
                )
            }
        }
    }
}

@Composable
private fun LinhaComInterruptor(rotulo: String, ligado: Boolean, aoTrocar: (Boolean) -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .defaultMinSize(minHeight = 56.dp)
            .clickable { aoTrocar(!ligado) }
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(rotulo, style = LumeTheme.typography.body, color = LumeTheme.colors.ink)
        Interruptor(ligado)
    }
}

/**
 * Interruptor desenhado à mão.
 *
 * O `Switch` do Material tem trilho de 52×32, polegar que cresce ao ligar e um
 * ícone opcional dentro. O design pede 44×26 com polegar de 20 e nada mais.
 * Reconfigurar o do Material exigiria sobrescrever oito cores e ainda deixaria a
 * geometria errada.
 */
@Composable
private fun Interruptor(ligado: Boolean) {
    Box(
        Modifier
            .size(width = 44.dp, height = 26.dp)
            .clip(LumeTheme.shapes.pill)
            .background(if (ligado) LumeTheme.colors.accent else LumeTheme.colors.trackOff)
            .padding(horizontal = 3.dp),
        contentAlignment = if (ligado) Alignment.CenterEnd else Alignment.CenterStart,
    ) {
        Box(
            Modifier
                .size(20.dp)
                .clip(CircleShape)
                .background(LumeTheme.colors.surfaceRaised),
        )
    }
}
