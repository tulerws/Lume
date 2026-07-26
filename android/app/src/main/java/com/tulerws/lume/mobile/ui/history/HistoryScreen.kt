package com.tulerws.lume.mobile.ui.history

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.tulerws.lume.mobile.R
import com.tulerws.lume.mobile.domain.HistoryEntry
import com.tulerws.lume.mobile.ui.components.BotaoContornado
import com.tulerws.lume.mobile.ui.components.Divisor
import com.tulerws.lume.mobile.ui.components.espacoParaBarra
import com.tulerws.lume.mobile.ui.components.EstadoVazio
import com.tulerws.lume.mobile.ui.components.RotuloDeSecao
import com.tulerws.lume.mobile.ui.horaDoRelogio
import com.tulerws.lume.mobile.ui.rotuloDoEvento
import com.tulerws.lume.mobile.ui.theme.LumeTheme
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId

/**
 * Tela de Histórico.
 *
 * **Sem barra de estado colorida nas linhas.** Histórico é registro do que já
 * aconteceu, não estado vivo; colorir cada linha faria esta tela competir com a de
 * Sessões pela mesma atenção, e a de Sessões é a que precisa ganhar.
 *
 * A tela busca quando fica visível — o protocolo não empurra histórico, porque as
 * entradas nascem dos mesmos eventos que já disparam `sessions.delta` e empurrá-las
 * também duplicaria tráfego por uma tela que quase nunca está aberta.
 */
@Composable
fun HistoryScreen(
    entradas: List<HistoryEntry>,
    carregando: Boolean,
    noTeto: Boolean,
    /** Código do protocolo da última falha, ou `null`. Ver `HistoryViewModel.falha`. */
    falha: String?,
    aoChegarNoFim: () -> Unit,
    aoTentarNovamente: () -> Unit,
    modifier: Modifier = Modifier,
) {
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
                    text = stringResource(R.string.historico_titulo),
                    style = LumeTheme.typography.title,
                    color = LumeTheme.colors.ink,
                )
            }
            Divisor()
        }

        when {
            entradas.isEmpty() && carregando -> Box(
                Modifier.fillMaxSize(),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = stringResource(R.string.historico_carregando),
                    style = LumeTheme.typography.label,
                    color = LumeTheme.colors.inkMuted,
                )
            }

            // A falha vem **antes** do vazio: "nada registrado ainda" seria
            // mentira quando o pedido nem chegou a ser feito.
            entradas.isEmpty() && falha != null -> EstadoVazio(
                titulo = stringResource(R.string.historico_falhou_titulo),
                corpo = stringResource(
                    when (falha) {
                        "desconectado" -> R.string.historico_falhou_desconectado
                        "invalid_request" -> R.string.historico_falhou_indisponivel
                        else -> R.string.historico_falhou_outro
                    },
                ),
                acao = {
                    Spacer(Modifier.height(10.dp))
                    BotaoContornado(
                        texto = stringResource(R.string.erro_tentar_novamente),
                        aoTocar = aoTentarNovamente,
                        modifier = Modifier.fillMaxWidth(0.6f),
                    )
                },
            )

            entradas.isEmpty() -> EstadoVazio(
                titulo = stringResource(R.string.historico_vazio_titulo),
                corpo = stringResource(R.string.historico_vazio_corpo),
            )

            else -> LazyColumn(Modifier.fillMaxSize(), contentPadding = espacoParaBarra()) {
                // O agrupamento por dia é calculado sobre a lista já ordenada, e
                // o cabeçalho entra quando o dia muda. Agrupar num `Map` perderia
                // a ordem, que é o único jeito de a paginação continuar de onde
                // parou.
                var diaAnterior: LocalDate? = null
                entradas.forEach { entrada ->
                    val dia = diaDe(entrada.createdAt)
                    if (dia != diaAnterior) {
                        diaAnterior = dia
                        item(key = "dia-${entrada.id}") {
                            CabecalhoDeDia(dia)
                        }
                    }
                    item(key = entrada.id) {
                        LinhaDeHistorico(entrada)
                        Divisor()
                    }
                }

                item(key = "rodape") {
                    if (falha != null) {
                        // Falha **depois** da primeira página: antes não aparecia
                        // nada. A lista simplesmente parava de crescer, sem
                        // mensagem e sem forma de tentar de novo, porque o rodapé
                        // já tinha disparado seu efeito e continuava composto.
                        Column(
                            Modifier
                                .fillMaxWidth()
                                .padding(horizontal = 16.dp, vertical = 20.dp),
                            horizontalAlignment = Alignment.CenterHorizontally,
                        ) {
                            Text(
                                text = stringResource(
                                    when (falha) {
                                        "desconectado" -> R.string.historico_falhou_desconectado
                                        "invalid_request" -> R.string.historico_falhou_indisponivel
                                        else -> R.string.historico_falhou_outro
                                    },
                                ),
                                style = LumeTheme.typography.label,
                                color = LumeTheme.colors.inkMuted,
                                textAlign = TextAlign.Center,
                            )
                            Spacer(Modifier.height(10.dp))
                            BotaoContornado(
                                texto = stringResource(R.string.erro_tentar_novamente),
                                aoTocar = aoTentarNovamente,
                                modifier = Modifier.fillMaxWidth(0.6f),
                            )
                        }
                    } else if (noTeto) {
                        // Diz o que é: teto do servidor, não fim do histórico. E
                        // não vira botão de carregar mais, que não carregaria nada.
                        Text(
                            text = stringResource(R.string.historico_teto),
                            style = LumeTheme.typography.label,
                            color = LumeTheme.colors.inkMuted,
                            textAlign = TextAlign.Center,
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(horizontal = 16.dp, vertical = 20.dp),
                        )
                    } else {
                        LaunchedEffectDeFim(aoChegarNoFim)
                    }
                }
            }
        }
    }
}

/**
 * Pede a próxima página quando o rodapé aparece.
 *
 * Ligado à composição do rodapé em vez de a um cálculo de índice de rolagem: o
 * rodapé só compõe quando entra na tela, o que já é a condição que interessa, e
 * não precisa saber quantos itens existem.
 */
@Composable
private fun LaunchedEffectDeFim(aoChegarNoFim: () -> Unit) {
    androidx.compose.runtime.LaunchedEffect(Unit) { aoChegarNoFim() }
}

@Composable
private fun CabecalhoDeDia(dia: LocalDate) {
    val hoje = LocalDate.now()
    val rotulo = when (dia) {
        hoje -> stringResource(R.string.historico_hoje)
        hoje.minusDays(1) -> stringResource(R.string.historico_ontem)
        // Fora de ontem e hoje, a data por extenso. O design só mostra os dois
        // primeiros casos porque os dados dele não vão além; a lista real vai.
        else -> dia.toString()
    }
    Box(
        Modifier
            .fillMaxWidth()
            .height(36.dp)
            .padding(horizontal = 16.dp),
        contentAlignment = Alignment.CenterStart,
    ) {
        RotuloDeSecao(rotulo)
    }
}

@Composable
private fun LinhaDeHistorico(entrada: HistoryEntry) {
    Row(
        Modifier
            .fillMaxWidth()
            .height(64.dp)
            .padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Text(
                // Rótulo do código do evento; `summary` só entra quando o evento é
                // desconhecido para este cliente, e aí a frase do backend é a
                // única informação que resta.
                text = rotuloDoEvento(entrada.event)?.let { stringResource(it) } ?: entrada.summary,
                style = LumeTheme.typography.titleSmall,
                color = LumeTheme.colors.ink,
            )
            Text(
                text = "${entrada.agentLabel} · ${entrada.project}",
                style = LumeTheme.typography.label,
                color = LumeTheme.colors.inkMuted,
            )
        }
        Text(
            text = horaDoRelogio(entrada.createdAt),
            style = LumeTheme.typography.label,
            color = LumeTheme.colors.inkMuted,
        )
    }
}

private fun diaDe(instante: Long): LocalDate =
    Instant.ofEpochMilli(instante).atZone(ZoneId.systemDefault()).toLocalDate()
