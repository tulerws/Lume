package com.tulerws.lume.mobile.ui

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import com.tulerws.lume.mobile.data.Pareador
import com.tulerws.lume.mobile.data.ResultadoDoPareamento
import com.tulerws.lume.mobile.data.remote.ConviteDePareamento
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

/**
 * A regressão do pareamento preso.
 *
 * O estado da tentativa morava no `ConnectionManager`, que é `@Singleton`, e
 * nada nunca o devolvia a `Ocioso`. Depois do primeiro pareamento a tela ficava
 * inutilizável: ao reabrir lia "concluído" no primeiro quadro e saía sozinha, e
 * depois de uma falha o leitor recusava toda leitura seguinte.
 *
 * Agora ele vive no ViewModel, que morre com a tela. Estes testes guardam as
 * duas propriedades que isso garante.
 */
/** Duplo de uma linha, porque `Pareador` tem um método. */
private class GerenteFalso(
    private val resultado: ResultadoDoPareamento = ResultadoDoPareamento.Pareado,
) : Pareador {
    var chamou = false
        private set

    override suspend fun parear(
        convite: ConviteDePareamento,
        nomeDoAparelho: String,
    ): ResultadoDoPareamento {
        chamou = true
        return resultado
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class PairViewModelTest {

    @Before
    fun antes() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun depois() = Dispatchers.resetMain()

    @Test
    fun `um ViewModel novo comeca ocioso`() {
        // O que a tela lê ao ser reaberta. Antes ela lia `Concluido`, herdado do
        // pareamento anterior, e saía no primeiro quadro — sem matar o processo
        // não havia como parear de novo.
        val vm = PairViewModel(GerenteFalso())
        assertEquals(EstadoDePareamento.Ocioso, vm.estado.value)
    }

    @Test
    fun `qr que nao e convite nao vira mensagem nem trava o leitor`() {
        // A câmera aponta para o mundo. Um QR de embalagem não pode virar aviso,
        // nem bloquear a leitura seguinte.
        val vm = PairViewModel(GerenteFalso())
        vm.aoLerQr("https://exemplo.com")
        assertEquals(null, vm.falhaDeLeitura.value)
        assertEquals(EstadoDePareamento.Ocioso, vm.estado.value)
    }

    @Test
    fun `qr corrompido vira mensagem e o estado segue ocioso`() {
        val vm = PairViewModel(GerenteFalso())
        vm.aoLerQr("lume://pair?v=1&f=curto&c=x&p=43140&h=&n=x")
        assertTrue(vm.falhaDeLeitura.value != null)
        // Ocioso: apontar a câmera de novo precisa funcionar.
        assertEquals(EstadoDePareamento.Ocioso, vm.estado.value)
    }

    @Test
    fun `tentarDeNovo devolve ao ocioso depois de uma falha`() {
        val vm = PairViewModel(GerenteFalso())
        vm.aoLerQr("lume://pair?v=2&f=x&c=x&p=1&h=&n=x")
        vm.tentarDeNovo()
        assertEquals(EstadoDePareamento.Ocioso, vm.estado.value)
        assertEquals(null, vm.falhaDeLeitura.value)
    }
}
