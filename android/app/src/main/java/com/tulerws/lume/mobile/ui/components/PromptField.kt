package com.tulerws.lume.mobile.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.unit.dp
import com.tulerws.lume.mobile.R
import com.tulerws.lume.mobile.ui.theme.LumeTheme

/** Limite do backend, em bytes. O contador aparece perto dele, não desde o começo. */
private const val LIMITE_DO_PROMPT = 16 * 1024
private const val LIMIAR_DO_CONTADOR = 15 * 1024

/**
 * Campo de prompt, ancorado ao rodapé da tela de sessão.
 *
 * Três decisões que o design fixa e que estão aqui:
 *
 * · **Desabilitado com o motivo escrito.** Quando a sessão está executando ou
 *   esperando permissão, o texto de dica é substituído pela razão — "Aguarde o
 *   agente terminar". Campo desabilitado sem explicação é interface muda, e o
 *   usuário fica sem saber se esperar ou se algo quebrou.
 *
 * · **O aviso de terminal vem antes, não depois.** Para Claude e Gemini, enviar
 *   prompt abre uma janela de terminal na máquina — com o usuário longe dela.
 *   Avisar depois de enviar seria avisar tarde demais. E o aviso **não** aparece
 *   para Codex aberto pelo Lume nem para sessão web, porque avisar sempre ensina
 *   a ignorar o aviso.
 *
 * · **O contador só aparece a partir de 15 KB.** Mostrar "0 / 16384" desde a
 *   primeira letra transforma um limite que quase ninguém alcança num assunto
 *   permanente.
 */
@Composable
fun PromptField(
    texto: String,
    aoMudar: (String) -> Unit,
    aoEnviar: () -> Unit,
    habilitado: Boolean,
    dica: String,
    modifier: Modifier = Modifier,
    avisoDeTerminal: Boolean = false,
) {
    val cores = LumeTheme.colors
    val bytes = texto.toByteArray().size
    val podeEnviar = habilitado && texto.isNotBlank() && bytes <= LIMITE_DO_PROMPT

    Column(modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp)) {
        if (avisoDeTerminal && habilitado) {
            Text(
                text = stringResource(R.string.prompt_aviso_terminal),
                style = LumeTheme.typography.label,
                color = cores.inkMuted,
            )
            Spacer(Modifier.height(8.dp))
        }

        Row(
            Modifier
                .fillMaxWidth()
                .defaultMinSize(minHeight = 48.dp)
                .clip(LumeTheme.shapes.field)
                .background(if (habilitado) cores.surfaceRaised else cores.fieldDisabled)
                .border(1.dp, cores.line, LumeTheme.shapes.field)
                .padding(start = 14.dp, end = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Box(Modifier.weight(1f)) {
                if (texto.isEmpty()) {
                    Text(
                        text = dica,
                        style = LumeTheme.typography.body,
                        color = cores.inkMuted,
                    )
                }
                if (habilitado) {
                    BasicTextField(
                        value = texto,
                        onValueChange = aoMudar,
                        textStyle = TextStyle(
                            fontFamily = LumeTheme.typography.body.fontFamily,
                            fontSize = LumeTheme.typography.body.fontSize,
                            color = cores.ink,
                        ),
                        cursorBrush = androidx.compose.ui.graphics.SolidColor(cores.accent),
                        keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
                            imeAction = ImeAction.Send,
                        ),
                        keyboardActions = androidx.compose.foundation.text.KeyboardActions(
                            onSend = { if (podeEnviar) aoEnviar() },
                        ),
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            }

            val rotuloEnviar = stringResource(R.string.prompt_enviar)
            Box(
                Modifier
                    .size(36.dp)
                    .clip(LumeTheme.shapes.field)
                    .background(if (podeEnviar) cores.accent else androidx.compose.ui.graphics.Color.Transparent)
                    .clickable(enabled = podeEnviar, onClick = aoEnviar)
                    .semantics {
                        contentDescription = rotuloEnviar
                        role = Role.Button
                    }
                    // 0,38 de opacidade quando não há o que enviar: o botão
                    // continua ocupando o lugar dele, para o campo não mudar de
                    // largura a cada letra digitada.
                    .alpha(if (podeEnviar) 1f else 0.38f),
                contentAlignment = Alignment.Center,
            ) {
                IconeEnviar(cor = if (podeEnviar) cores.surface else cores.ink)
            }
        }

        if (bytes >= LIMIAR_DO_CONTADOR) {
            Spacer(Modifier.height(6.dp))
            Text(
                text = "$bytes / $LIMITE_DO_PROMPT",
                style = LumeTheme.typography.label,
                color = if (bytes > LIMITE_DO_PROMPT) {
                    LumeTheme.states.failed.on
                } else {
                    cores.inkMuted
                },
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}
