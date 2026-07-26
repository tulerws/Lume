package com.tulerws.lume.mobile.data.remote

import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import com.tulerws.lume.mobile.data.remote.protocol.MensagemDoServidor
import com.tulerws.lume.mobile.data.remote.protocol.SUBPROTOCOLO
import com.tulerws.lume.mobile.data.remote.protocol.lerMensagem
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import javax.inject.Inject
import javax.inject.Singleton

/** Onde tentar conectar. */
data class Endereco(val host: String, val porta: Int) {

    /**
     * URL do WebSocket.
     *
     * **Os colchetes de IPv6 são acrescentados aqui**, e não vêm no `h=` do QR.
     * O documento é explícito: no QR o endereço é valor de query, onde dois-pontos
     * é permitido, e quem monta a autoridade da URL é o aplicativo. Se o QR já
     * trouxesse colchetes o resultado seria `wss://[[2001:db8::1]]:43140`.
     */
    fun url(): String {
        val autoridade = if (host.contains(':') && !host.startsWith("[")) "[$host]" else host
        return "wss://$autoridade:$porta/lume"
    }
}

/** Como esta conexão se apresenta: token de aparelho já registrado, ou código de pareamento. */
sealed interface Credencial {
    /** Aparelho já pareado. `Authorization: Bearer <token>`. */
    data class Token(val valor: String) : Credencial

    /** Primeiro contato. `X-Lume-Pairing-Code: <codigo>`, consumido no aperto de mão. */
    data class CodigoDePareamento(val valor: String) : Credencial
}

/** O que acontece num canal. */
sealed interface EventoDoCanal {
    data class Aberto(val canal: CanalAberto) : EventoDoCanal
    data class Recebida(val mensagem: MensagemDoServidor) : EventoDoCanal

    /**
     * Encerramento ou falha.
     *
     * [httpStatus] só existe quando o servidor recusou **antes** do upgrade — 401
     * para credencial inválida, 426 para subprotocolo não suportado. É a diferença
     * entre "não deu para conectar" e "você não é bem-vindo", e a interface precisa
     * poder contar as duas de formas diferentes.
     */
    data class Encerrado(
        val httpStatus: Int?,
        val motivo: String?,
    ) : EventoDoCanal
}

/** Um canal vivo. */
class CanalAberto internal constructor(private val socket: WebSocket) {
    fun enviar(texto: String): Boolean = socket.send(texto)

    /** 1000 é encerramento normal — o servidor não deve tratar como queda. */
    fun fechar() {
        socket.close(1000, null)
    }
}

/**
 * Abre canais WSS fixados por fingerprint.
 *
 * Só isso: abrir, entregar eventos e fechar. Quem decide **quando** reconectar,
 * em que endereço e com qual credencial é o `ConnectionManager` — separar as duas
 * coisas é o que permite testar a política de reconexão sem rede, e o que faz
 * mover a conexão para dentro de um serviço em primeiro plano na v2 não tocar
 * nesta classe.
 */
@Singleton
class LumeClient @Inject constructor() {

    /**
     * Abre um canal e emite tudo que acontece nele.
     *
     * O fluxo termina quando o canal termina. Cancelar a coleta fecha o socket
     * pelo `awaitClose`, que é o que faz `disconnect()` em `onStop` não deixar
     * conexão órfã.
     */
    fun conectar(
        endereco: Endereco,
        credencial: Credencial,
        fingerprint: ByteArray,
    ): Flow<EventoDoCanal> = callbackFlow {
        val cliente = clienteFixado(fingerprint)

        val requisicao = Request.Builder()
            .url(endereco.url())
            // Negociado no aperto de mão. Servidor que não suporte responde 426
            // com a lista do que suporta, e aí a mensagem certa para o usuário é
            // "atualize o Lume no computador" — não "falha de conexão".
            .header("Sec-WebSocket-Protocol", SUBPROTOCOLO)
            .apply {
                when (credencial) {
                    // Se os dois cabeçalhos fossem juntos, o token venceria no
                    // servidor. Mandar um só evita consumir um código de
                    // pareamento à toa num aparelho que já é alguém.
                    is Credencial.Token -> header("Authorization", "Bearer ${credencial.valor}")
                    is Credencial.CodigoDePareamento ->
                        header("X-Lume-Pairing-Code", credencial.valor)
                }
            }
            .build()

        val ouvinte = object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                trySend(EventoDoCanal.Aberto(CanalAberto(webSocket)))
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                // Envelope torto é violação de protocolo, e o servidor mantém a
                // conexão viva nesse caso. Aqui também: um envelope ilegível não
                // justifica derrubar o canal e refazer TLS e snapshot.
                val mensagem = runCatching { lerMensagem(text) }.getOrNull() ?: return
                trySend(EventoDoCanal.Recebida(mensagem))
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(1000, null)
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                trySend(EventoDoCanal.Encerrado(null, reason.ifBlank { null }))
                close()
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                trySend(EventoDoCanal.Encerrado(response?.code, t.message))
                close()
            }
        }

        val socket = cliente.newWebSocket(requisicao, ouvinte)

        awaitClose {
            socket.cancel()
            // O pool e o executor são desta conexão: sem isto, cada tentativa de
            // reconexão deixaria threads para trás, e a reconexão é em laço.
            cliente.dispatcher.executorService.shutdown()
            cliente.connectionPool.evictAll()
        }
    }
}
