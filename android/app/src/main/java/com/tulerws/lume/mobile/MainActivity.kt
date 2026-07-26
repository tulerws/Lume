package com.tulerws.lume.mobile

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import dagger.hilt.android.AndroidEntryPoint
import com.tulerws.lume.mobile.ui.LumeApp

/**
 * A única activity.
 *
 * `enableEdgeToEdge` faz o conteúdo passar por baixo das barras do sistema, e cada
 * tela reserva o espaço com `statusBarsPadding` e `navigationBarsPadding`. É por
 * isso que a barra de status e a barra de gestos que aparecem no arquivo de design
 * **não** são desenhadas aqui: aquelas são o sistema do aparelho, não conteúdo do
 * aplicativo. Desenhá-las produziria duas barras de status na tela de verdade.
 *
 * `FLAG_SECURE` ainda não está aqui. Ele pertence ao mesmo conjunto que biometria
 * e credencial no Keystore, e este trabalho é de interface — ligá-lo agora
 * atrapalharia a captura de tela que é justamente como o resultado vai ser
 * conferido contra o design.
 */
@AndroidEntryPoint
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            LumeApp()
        }
    }
}
