package lume.ai.domain

import androidx.compose.runtime.Immutable

/**
 * O desktop pareado, do ponto de vista do celular.
 *
 * Não é espelho de `RemoteDevice`: aquele tipo descreve o **celular** visto pelo
 * desktop. Este descreve o **desktop** visto pelo celular, e os dois lados guardam
 * coisas diferentes. O nome aparece na faixa de erro ("Sem conexão com
 * marcos-desktop") e em Ajustes.
 */
@Immutable
data class PairedDesktop(
    val nome: String,
    /** Rótulo já formatado, como o design mostra: "12 jul". */
    val pareadoEm: String,
)

/**
 * Estado da conexão com o desktop.
 *
 * `docs/ANDROID.md` lista quatro estados sem dados. [Desconectado] carrega
 * [ultimoContato] porque a tela de sem-conexão do design precisa dizer "Último
 * estado conhecido · há 6 min" — sem isso, uma lista parada por queda de conexão
 * teria exatamente a mesma aparência de uma lista de agentes parados, que é a
 * confusão mais provável desta interface.
 */
@Immutable
sealed interface ConnectionState {

    /** Tentando alcançar o desktop. Ponto em `waitingForInput`. */
    data object Conectando : ConnectionState

    /** Canal vivo. Ponto em `completed`. */
    data object Conectado : ConnectionState

    /**
     * Sem canal, mostrando cache.
     *
     * @param ultimoContato rótulo relativo do último instante em que houve dado
     *   fresco, ou `null` quando nunca houve — caso em que a tela não deve
     *   prometer um "último estado" que não existe.
     */
    data class Desconectado(val ultimoContato: String? = null) : ConnectionState

    /**
     * Falha que o usuário precisa ver e sobre a qual pode agir.
     *
     * Separado de [Desconectado] de propósito: desconectado é situação, erro é
     * evento. O primeiro esmaece a lista e segue mostrando cache; o segundo ocupa
     * a tela e oferece "Tentar de novo".
     */
    data class Erro(val motivo: String) : ConnectionState
}
