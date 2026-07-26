package lume.ai.ui

import androidx.annotation.StringRes
import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import lume.ai.R
import lume.ai.domain.ActivityStatus
import lume.ai.domain.AgentSession
import lume.ai.domain.ConnectionState
import lume.ai.domain.HistoryEvent
import lume.ai.domain.PermissionAction
import lume.ai.domain.RiskLevel
import lume.ai.domain.SessionStatus
import lume.ai.ui.components.EstadoDoMascote
import lume.ai.ui.theme.LumeTheme
import lume.ai.ui.theme.StateColor
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/**
 * Tradução de domínio para apresentação.
 *
 * Tudo que converte um valor do protocolo em algo visível mora aqui, e em nenhum
 * componente. O motivo é o mesmo em todos os casos: cada uma destas funções
 * precisa dar a mesma resposta em toda tela, e uma decisão repetida em cinco
 * arquivos é uma decisão que vai divergir em quatro.
 */

/** Rótulo de estado — do enum, nunca de `statusLabel`. Ver `AgentSession.statusLabel`. */
@StringRes
fun rotuloDoEstado(status: SessionStatus): Int = when (status) {
    SessionStatus.Running -> R.string.estado_executando
    SessionStatus.PermissionRequired -> R.string.estado_aguardando_permissao
    SessionStatus.WaitingForInput -> R.string.estado_aguardando_resposta
    SessionStatus.Completed -> R.string.estado_concluido
    SessionStatus.Failed -> R.string.estado_falhou
}

@Composable
@ReadOnlyComposable
fun coresDoEstado(status: SessionStatus): StateColor = with(LumeTheme.states) {
    when (status) {
        SessionStatus.Running -> running
        SessionStatus.PermissionRequired -> permissionRequired
        SessionStatus.WaitingForInput -> waitingForInput
        SessionStatus.Completed -> completed
        SessionStatus.Failed -> failed
    }
}

/**
 * Cor do ponto de uma linha de atividade.
 *
 * `waiting` cai em `waitingForInput` e não em `running`: são estados diferentes no
 * protocolo, e usar a mesma cor apagaria a diferença sem ganhar nada.
 */
@Composable
@ReadOnlyComposable
fun coresDaAtividade(status: ActivityStatus): StateColor = with(LumeTheme.states) {
    when (status) {
        ActivityStatus.Running -> running
        ActivityStatus.Completed -> completed
        ActivityStatus.Failed -> failed
        ActivityStatus.Waiting -> waitingForInput
    }
}

/**
 * Rótulo de um evento de histórico.
 *
 * Devolve `null` para [HistoryEvent.Desconhecido] — evento que este aplicativo
 * não conhece não tem rótulo próprio, e a tela cai no `summary` que o backend
 * mandou. Inventar um texto genérico ali ("Atividade") apagaria a informação que
 * o servidor de fato enviou.
 */
@StringRes
fun rotuloDoEvento(evento: HistoryEvent): Int? = when (evento) {
    HistoryEvent.Completed -> R.string.evento_concluido
    HistoryEvent.Failed -> R.string.evento_falhou
    HistoryEvent.PermissionAllowed -> R.string.permissao_concedida
    HistoryEvent.PermissionDenied -> R.string.permissao_recusada
    HistoryEvent.Desconhecido -> null
}

@StringRes
fun rotuloDoRisco(risco: RiskLevel): Int = when (risco) {
    RiskLevel.High -> R.string.risco_alto
    RiskLevel.Medium -> R.string.risco_medio
    RiskLevel.Low -> R.string.risco_baixo
}

/**
 * Cor do risco.
 *
 * Alto usa a cor de falha e médio a de permissão. Não são cores novas: o risco
 * fala da mesma coisa que o estado — quanto isto pode dar errado —, e inventar
 * uma escala paralela obrigaria o usuário a aprender dois vocabulários de cor.
 */
@Composable
@ReadOnlyComposable
fun coresDoRisco(risco: RiskLevel): StateColor = with(LumeTheme.states) {
    when (risco) {
        RiskLevel.High -> failed
        RiskLevel.Medium -> permissionRequired
        RiskLevel.Low -> completed
    }
}

@StringRes
fun rotuloDaAcao(acao: PermissionAction): Int = when (acao) {
    PermissionAction.AllowOnce -> R.string.permissao_permitir_uma_vez
    PermissionAction.AllowSession -> R.string.permissao_permitir_sessao
    PermissionAction.Deny -> R.string.permissao_recusar
    // Nunca chega aqui: `acoesRespondiveis()` filtra `OpenSource` antes. O ramo
    // existe porque o enum espelha o protocolo, e um `else` esconderia a adição
    // de um valor novo lá.
    PermissionAction.OpenSource -> R.string.permissao_somente_observacao
}

@StringRes
fun rotuloDaConexao(estado: ConnectionState): Int = when (estado) {
    is ConnectionState.Conectado -> R.string.conexao_conectado
    is ConnectionState.Conectando -> R.string.conexao_conectando
    is ConnectionState.Desconectado, is ConnectionState.Erro -> R.string.conexao_sem
}

@Composable
@ReadOnlyComposable
fun coresDaConexao(estado: ConnectionState): StateColor = with(LumeTheme.states) {
    when (estado) {
        is ConnectionState.Conectado -> completed
        is ConnectionState.Conectando -> waitingForInput
        is ConnectionState.Desconectado, is ConnectionState.Erro -> idle
    }
}

/**
 * Estado do mascote a partir da conexão e das sessões, **nessa ordem**.
 *
 * A precedência não é arbitrária: "estou conectado?" é uma pergunta que só existe
 * no celular, e sem conexão nenhuma informação sobre agentes é confiável. Por isso
 * a conexão decide primeiro; só depois entra o estado agregado, e aí vale a mesma
 * urgência que ordena a lista.
 */
fun estadoDoMascote(conexao: ConnectionState, sessoes: List<AgentSession>): EstadoDoMascote {
    if (conexao !is ConnectionState.Conectado) return EstadoDoMascote.Dormindo
    val estados = sessoes.map { it.status }
    return when {
        SessionStatus.PermissionRequired in estados -> EstadoDoMascote.PedindoPermissao
        SessionStatus.Running in estados -> EstadoDoMascote.Executando
        SessionStatus.WaitingForInput in estados -> EstadoDoMascote.Esperando
        SessionStatus.Failed in estados -> EstadoDoMascote.Falhou
        SessionStatus.Completed in estados -> EstadoDoMascote.Concluido
        else -> EstadoDoMascote.Desperto
    }
}

/**
 * Resumo agregado sob o mascote: "1 pedindo permissão · 2 executando".
 *
 * Só as duas categorias que exigem atenção entram. Listar concluídas e falhadas
 * junto encheria a linha com o que já passou, e a linha existe para dizer o que
 * está acontecendo agora.
 */
@Composable
fun resumoDasSessoes(sessoes: List<AgentSession>): String {
    val permissao = sessoes.count { it.status == SessionStatus.PermissionRequired }
    val executando = sessoes.count { it.status == SessionStatus.Running }
    val partes = buildList {
        if (permissao > 0) {
            add(pluralStringResource(R.plurals.sessoes_resumo_permissao, permissao, permissao))
        }
        if (executando > 0) {
            add(pluralStringResource(R.plurals.sessoes_resumo_executando, executando, executando))
        }
    }
    return if (partes.isEmpty()) stringResource(R.string.sessoes_resumo_ocioso) else partes.joinToString(" · ")
}

/**
 * Tempo relativo curto: "há 2 min", "há 1 h".
 *
 * Arredonda para baixo e nunca mostra segundos. Um contador que muda a cada
 * segundo numa lista de seis linhas rouba atenção de quem precisa ver qual delas
 * está pedindo permissão.
 */
@Composable
fun tempoRelativo(instante: Long, agora: Long = System.currentTimeMillis()): String {
    val minutos = ((agora - instante).coerceAtLeast(0)) / 60_000
    return when {
        minutos < 1 -> stringResource(R.string.tempo_agora)
        minutos < 60 -> stringResource(R.string.tempo_minutos, minutos.toInt())
        minutos < 60 * 24 -> stringResource(R.string.tempo_horas, (minutos / 60).toInt())
        else -> stringResource(R.string.tempo_dias, (minutos / (60 * 24)).toInt())
    }
}

private val formatoDeHora = DateTimeFormatter.ofPattern("HH:mm")

/**
 * Hora do relógio, para atividade e histórico.
 *
 * Formato de 24 horas fixo, como o design mostra. Seguir a preferência do sistema
 * traria "2:26 PM" para parte dos usuários, e a coluna da direita do histórico
 * tem largura pensada para cinco caracteres.
 */
fun horaDoRelogio(instante: Long): String =
    Instant.ofEpochMilli(instante).atZone(ZoneId.systemDefault()).format(formatoDeHora)
