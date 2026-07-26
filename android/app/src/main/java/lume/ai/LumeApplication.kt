package lume.ai

import android.app.Application
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import dagger.hilt.android.HiltAndroidApp
import lume.ai.data.ConnectionManager
import javax.inject.Inject

/**
 * Raiz do grafo de injeção, e dona do ciclo de vida da conexão.
 *
 * O observador está no `ProcessLifecycleOwner`, e não na activity, de propósito:
 * girar a tela destrói e recria a activity, e uma conexão presa a ela cairia e
 * refaria TLS mais snapshot a cada rotação. O `ProcessLifecycleOwner` só chama
 * `onStop` quando o **aplicativo inteiro** vai para segundo plano.
 *
 * A consequência precisa estar clara na interface, e não só aqui: **fechou o
 * aplicativo, parou de receber.** Não há serviço em primeiro plano nem push na
 * v1 — isso é assunto da v2, e o `ConnectionManager` já está desenhado para ser
 * movido para dentro de um serviço sem tocar em nada acima dele.
 */
@HiltAndroidApp
class LumeApplication : Application() {

    @Inject
    lateinit var conexao: ConnectionManager

    override fun onCreate() {
        super.onCreate()
        ProcessLifecycleOwner.get().lifecycle.addObserver(
            object : DefaultLifecycleObserver {
                override fun onStart(owner: LifecycleOwner) {
                    conexao.conectar()
                }

                override fun onStop(owner: LifecycleOwner) {
                    // Fecha com 1000 (encerramento normal): o servidor não deve
                    // registrar isto como queda, e o aparelho não deve segurar
                    // uma thread do outro lado enquanto está em segundo plano.
                    conexao.desconectar()
                }
            },
        )
    }
}
