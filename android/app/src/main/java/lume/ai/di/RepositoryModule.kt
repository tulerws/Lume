package lume.ai.di

import dagger.Binds
import dagger.Module
import dagger.hilt.InstallIn
import dagger.hilt.components.SingletonComponent
import lume.ai.data.repo.HistoryRepository
import lume.ai.data.repo.PairingRepository
import lume.ai.data.repo.RemoteHistoryRepository
import lume.ai.data.repo.RemotePairingRepository
import lume.ai.data.repo.RemoteSessionRepository
import lume.ai.data.repo.SessionRepository
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
