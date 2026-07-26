package com.tulerws.lume.mobile.data

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import com.tulerws.lume.mobile.ui.theme.ThemeMode
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Preferências do aplicativo, em memória.
 *
 * Tema e avisos, que é o que a tela de Ajustes do design oferece.
 *
 * **Não persiste.** Fechar o aplicativo devolve tudo ao padrão. O lugar disto é o
 * DataStore, e o DataStore é infraestrutura — que ficou de fora deste trabalho por
 * decisão explícita. A forma aqui já é a definitiva (`StateFlow` lido pela
 * interface), então trocar o miolo por leitura e escrita em disco não toca tela
 * nenhuma.
 *
 * Os avisos ficam guardados e não fazem nada: não há notificação na v1, e o
 * interruptor existe no design porque a decisão de alertar já é do backend
 * (`should_notify`) — o que falta é só o canal.
 */
@Singleton
class PreferenciasEmMemoria @Inject constructor() {

    private val _tema = MutableStateFlow(ThemeMode.System)
    val tema: StateFlow<ThemeMode> = _tema.asStateFlow()

    private val _avisarPermissao = MutableStateFlow(true)
    val avisarPermissao: StateFlow<Boolean> = _avisarPermissao.asStateFlow()

    private val _avisarConclusao = MutableStateFlow(false)
    val avisarConclusao: StateFlow<Boolean> = _avisarConclusao.asStateFlow()

    fun definirTema(modo: ThemeMode) {
        _tema.value = modo
    }

    fun definirAvisoDePermissao(ligado: Boolean) {
        _avisarPermissao.value = ligado
    }

    fun definirAvisoDeConclusao(ligado: Boolean) {
        _avisarConclusao.value = ligado
    }
}
