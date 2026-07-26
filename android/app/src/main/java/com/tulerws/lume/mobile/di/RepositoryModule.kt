package com.tulerws.lume.mobile.di

import dagger.Binds
import dagger.Module
import dagger.hilt.InstallIn
import dagger.hilt.components.SingletonComponent
import com.tulerws.lume.mobile.data.ConnectionManager
import com.tulerws.lume.mobile.data.Pareador
import com.tulerws.lume.mobile.data.repo.HistoryRepository
import com.tulerws.lume.mobile.data.repo.PairingRepository
import com.tulerws.lume.mobile.data.repo.RemoteHistoryRepository
import com.tulerws.lume.mobile.data.repo.RemotePairingRepository
import com.tulerws.lume.mobile.data.repo.RemoteSessionRepository
import com.tulerws.lume.mobile.data.repo.SessionRepository
import javax.inject.Singleton

/**
 * Onde as interfaces são ligadas à rede.
 *
 * Este arquivo é a costura inteira entre a interface e o mundo — e foi o que
 * tornou a troca do falso pelo real uma edição de três linhas, sem tocar em
 * ViewModel, tela ou componente nenhum. É o retorno concreto de ter escolhido as
 * assinaturas definitivas antes de existir protocolo rodando.
 *
 * **Não há mais implementação falsa no aplicativo.** Os dados do design vivem em
 * `data/fake/DadosDoDesign.kt` e são consumidos apenas pelos `@Preview`, que não
 * passam pelo Hilt. Um repositório falso ligado aqui poderia dizer "Conectado"
 * sobre dado inventado, e a tela de Sessões foi desenhada inteira em torno de
 * esse indicador nunca mentir.
 */
@Module
@InstallIn(SingletonComponent::class)
abstract class RepositoryModule {

    /** A tela de pareamento vê só isto do gerente de conexão. */
    @Binds
    @Singleton
    abstract fun pareador(real: ConnectionManager): Pareador

    @Binds
    @Singleton
    abstract fun sessionRepository(real: RemoteSessionRepository): SessionRepository

    @Binds
    @Singleton
    abstract fun historyRepository(real: RemoteHistoryRepository): HistoryRepository

    @Binds
    @Singleton
    abstract fun pairingRepository(real: RemotePairingRepository): PairingRepository
}
