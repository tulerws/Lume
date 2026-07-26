package com.tulerws.lume.mobile.ui.pair

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import com.tulerws.lume.mobile.R
import com.tulerws.lume.mobile.ui.components.BotaoPreenchido
import com.tulerws.lume.mobile.ui.components.Divisor
import com.tulerws.lume.mobile.ui.components.IconeVoltar
import com.tulerws.lume.mobile.ui.components.RotuloDeSecao
import com.tulerws.lume.mobile.ui.theme.LumeTheme

/**
 * Entrada manual de **endereço e porta**. Nada além.
 *
 * O nome "caminho manual" já significou duas coisas neste projeto, e uma delas
 * não existe. `docs/REMOTE-CONTROL.md`, seção *O caminho manual é endereço, e
 * nada além*, é normativo: **não há campo de código aqui, e não haverá.**
 *
 * O motivo não é de interface. Autenticar exige o código de pareamento, e
 * verificar um certificado autoassinado exige o fingerprint; os dois só existem
 * dentro do QR, e o desktop deliberadamente nunca os põe em texto — pôr o código
 * na tela o colocaria também na memória do JavaScript do webview. Digitar
 * endereço e código sem conferir a chave devolveria a um atacante da mesma rede a
 * posição de intermediário permanente que o campo `f=` do QR existe para negar.
 *
 * Logo, esta tela serve a dois casos, e **nos dois o QR já foi lido**:
 *
 * · **QR lido, endereço inútil** — `h=` veio vazio, ou traz endereços de uma
 *   interface que o celular não alcança. Código e fingerprint vieram do QR;
 *   falta só onde conectar.
 * · **Aparelho pareado, IP mudou** — token e fingerprint estão guardados desde o
 *   pareamento, e o mDNS não atravessa VPN.
 *
 * A validação é de formato e é local:
 *
 * · **Endereço** não vazio, e não validado como IP: o campo aceita nome de
 *   máquina, IPv4 e IPv6, e um regex que tentasse cobrir os três recusaria
 *   entrada legítima.
 * · **Porta** entre 1 e 65535.
 */
@Composable
fun ManualEntryScreen(
    aoVoltar: () -> Unit,
    aoConectar: (endereco: String, porta: Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    var endereco by remember { mutableStateOf("") }
    var porta by remember { mutableStateOf("43140") }
    var tentouEnviar by remember { mutableStateOf(false) }

    val portaValida = porta.toIntOrNull()?.let { it in 1..65535 } == true
    val enderecoValido = endereco.isNotBlank()

    Column(
        modifier
            .fillMaxSize()
            .background(LumeTheme.colors.surface)
            .imePadding(),
    ) {
        Column(Modifier.statusBarsPadding()) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .height(56.dp)
                    .padding(horizontal = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                val rotuloVoltar = stringResource(R.string.acao_voltar)
                Box(
                    Modifier
                        .size(48.dp)
                        .clickable(onClick = aoVoltar)
                        .semantics {
                            contentDescription = rotuloVoltar
                            role = Role.Button
                        },
                    contentAlignment = Alignment.Center,
                ) {
                    IconeVoltar(LumeTheme.colors.ink)
                }
                Text(
                    text = stringResource(R.string.manual_titulo),
                    style = LumeTheme.typography.title,
                    color = LumeTheme.colors.ink,
                )
            }
            Divisor()
        }

        Column(
            Modifier
                .fillMaxSize()
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Campo(
                rotulo = stringResource(R.string.manual_endereco),
                valor = endereco,
                aoMudar = { endereco = it },
                erro = if (tentouEnviar && !enderecoValido) {
                    stringResource(R.string.manual_erro_endereco)
                } else {
                    null
                },
                // O endereço pode ser IPv6, e IPv6 tem dois-pontos e letras. Teclado
                // de texto, não numérico.
                tipo = KeyboardType.Uri,
            )
            Campo(
                rotulo = stringResource(R.string.manual_porta),
                valor = porta,
                aoMudar = { porta = it.filter { c -> c.isDigit() }.take(5) },
                erro = if (tentouEnviar && !portaValida) {
                    stringResource(R.string.manual_erro_porta)
                } else {
                    null
                },
                tipo = KeyboardType.Number,
            )
            // Diz para que serve, antes do botão. Sem esta linha, uma tela com
            // "Endereço", "Porta" e "Conectar" parece um caminho de pareamento —
            // e ela não é um, nem pode ser.
            Text(
                text = stringResource(R.string.manual_explicacao),
                style = LumeTheme.typography.label,
                color = LumeTheme.colors.inkMuted,
            )

            Spacer(Modifier.height(8.dp))
            BotaoPreenchido(
                texto = stringResource(R.string.manual_conectar),
                aoTocar = {
                    tentouEnviar = true
                    if (enderecoValido && portaValida) {
                        aoConectar(endereco.trim(), porta.toInt())
                    }
                },
            )
        }
    }
}

@Composable
private fun Campo(
    rotulo: String,
    valor: String,
    aoMudar: (String) -> Unit,
    erro: String?,
    tipo: KeyboardType,
) {
    val cores = LumeTheme.colors
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        RotuloDeSecao(rotulo)
        Box(
            Modifier
                .fillMaxWidth()
                .height(48.dp)
                .clip(LumeTheme.shapes.field)
                .background(cores.surfaceRaised)
                .border(
                    1.dp,
                    // A borda vermelha é o segundo sinal; o texto abaixo é o
                    // primeiro. Cor sozinha nunca carrega o erro.
                    if (erro != null) LumeTheme.states.failed.on else cores.line,
                    LumeTheme.shapes.field,
                )
                .padding(horizontal = 14.dp),
            contentAlignment = Alignment.CenterStart,
        ) {
            BasicTextField(
                value = valor,
                onValueChange = aoMudar,
                singleLine = true,
                textStyle = TextStyle(
                    fontFamily = LumeTheme.typography.body.fontFamily,
                    fontSize = LumeTheme.typography.body.fontSize,
                    color = cores.ink,
                ),
                cursorBrush = SolidColor(cores.accent),
                keyboardOptions = KeyboardOptions(keyboardType = tipo),
                modifier = Modifier.fillMaxWidth(),
            )
        }
        if (erro != null) {
            Text(
                text = erro,
                style = LumeTheme.typography.label,
                color = LumeTheme.states.failed.on,
            )
        }
    }
}
