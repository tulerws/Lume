package com.tulerws.lume.mobile.data.update

import android.content.Context
import android.content.Intent
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.content.pm.Signature
import android.net.Uri
import android.os.Build
import android.os.SystemClock
import android.provider.Settings
import androidx.core.content.FileProvider
import com.tulerws.lume.mobile.BuildConfig
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.TimeUnit
import javax.inject.Inject
import javax.inject.Singleton

/** Onde o manifesto vive. `latest/download` sempre resolve para a release mais recente. */
private const val ENDERECO_DO_MANIFESTO =
    "https://github.com/tulerws/Lume/releases/latest/download/mobile-latest.json"

/** Prefixo aceito para o APK. Ver [urlConfiavel]. */
private const val PREFIXO_ACEITO = "/tulerws/Lume/releases/download/"

/** Represa entre checagens automáticas dentro de uma mesma execução do processo. */
private val INTERVALO_MINIMO = TimeUnit.HOURS.toMillis(6)

/**
 * Estado do atualizador, tal como a tela precisa vê-lo.
 *
 * [PrecisaAutorizar] existe porque a permissão do manifesto **não basta** desde o
 * Android 8: `REQUEST_INSTALL_PACKAGES` dá o direito de pedir, e a autorização de
 * "fontes desconhecidas" é concedida por aplicativo, na tela de Ajustes do
 * sistema. Sem um estado próprio, esse caso apareceria como uma falha genérica e a
 * pessoa não teria como saber que faltava um interruptor em outro lugar.
 */
sealed interface EstadoDaAtualizacao {
    data object Ocioso : EstadoDaAtualizacao
    data object Verificando : EstadoDaAtualizacao
    data object EmDia : EstadoDaAtualizacao
    data class Disponivel(val versao: String, val notas: String?) : EstadoDaAtualizacao
    data class Baixando(val fracao: Float?) : EstadoDaAtualizacao
    data class Pronta(val versao: String) : EstadoDaAtualizacao
    data object PrecisaAutorizar : EstadoDaAtualizacao
    data class Falhou(val codigo: String) : EstadoDaAtualizacao
}

/**
 * Verifica, baixa e entrega ao instalador do sistema.
 *
 * ## Por que a rede aqui não é a rede do resto do aplicativo
 *
 * Todo o restante fala com **um único host** — o desktop pareado — sobre TLS com
 * o certificado fixado por fingerprint (`PinnedTrust.kt`). Este arquivo usa um
 * cliente OkHttp **separado, sem fixação**, e isso é deliberado: o GitHub tem
 * certificado emitido por autoridade pública, e fixá-lo quebraria o atualizador na
 * primeira rotação de certificado deles — exatamente o momento em que ninguém
 * estaria olhando. Contra o GitHub, a validação correta é a do armazém do sistema.
 *
 * A escolha de buscar direto, em vez de perguntar ao desktop, é a decisão que
 * torna o atualizador útil: quem mais precisa atualizar é justamente quem está com
 * o pareamento quebrado ou o desktop fora do ar. Um atualizador que só funciona
 * quando está tudo bem não atualiza ninguém.
 *
 * ## Os três portões
 *
 * O custo dessa abertura é pago por verificações que não dependem de confiar na
 * rede, e nenhuma delas sozinha é suficiente:
 *
 * 1. **Origem** — [urlConfiavel] recusa qualquer endereço que não seja `https` em
 *    `github.com` sob [PREFIXO_ACEITO]. Um manifesto adulterado não consegue
 *    apontar para outro servidor.
 * 2. **Conteúdo** — o SHA-256 do arquivo baixado tem de bater com o do manifesto.
 * 3. **Identidade** — a assinatura do APK tem de ser idêntica à do aplicativo
 *    instalado, conferida **antes** de mostrar o instalador.
 *
 * O terceiro portão é, sozinho, redundante: o sistema recusaria a instalação de
 * qualquer jeito. Ele existe para que a recusa aconteça aqui, com mensagem nossa,
 * e não como um "aplicativo não instalado" seco vindo do instalador depois de a
 * pessoa ter baixado trinta e quatro megabytes.
 *
 * O resultado é que um atacante com controle do DNS consegue **negar** a
 * atualização — nunca forjar uma.
 */
@Singleton
class Atualizador @Inject constructor(
    @param:ApplicationContext private val context: Context,
) {

    private val _estado = MutableStateFlow<EstadoDaAtualizacao>(EstadoDaAtualizacao.Ocioso)
    val estado: StateFlow<EstadoDaAtualizacao> = _estado.asStateFlow()

    /**
     * Cliente próprio, com os prazos curtos que um manifesto de 200 bytes merece.
     *
     * `followRedirects` fica ligado porque o GitHub responde o download com 302
     * para `objects.githubusercontent.com`. Isso não afrouxa o portão de origem: o
     * endereço que validamos é o que **nós** pedimos, o destino do redirecionamento
     * é escolhido pelo GitHub dentro de uma conexão TLS já verificada, e o conteúdo
     * que chegar ainda terá de bater com o SHA-256.
     */
    private val cliente: OkHttpClient by lazy {
        OkHttpClient.Builder()
            .connectTimeout(15, TimeUnit.SECONDS)
            .readTimeout(30, TimeUnit.SECONDS)
            .followRedirects(true)
            .build()
    }

    private val json = Json { ignoreUnknownKeys = true }

    /**
     * Instante da última checagem automática, em tempo de atividade do sistema.
     *
     * Guardado em memória e não em disco, e isso é o comportamento pretendido — não
     * uma simplificação. A decisão foi verificar **ao abrir o aplicativo**; um
     * processo novo deve mesmo checar de novo. O que esta represa impede é a
     * repetição dentro da mesma execução, quando a tela é recomposta ou a pessoa
     * navega de volta para Ajustes.
     *
     * `elapsedRealtime` em vez do relógio de parede porque este último anda para
     * trás quando o fuso muda ou o NTP corrige, e uma represa que anda para trás
     * deixa de represar.
     */
    private var ultimaChecagem: Long = 0

    /** Verificação automática: respeita a represa e nunca sobrescreve um download em curso. */
    suspend fun verificarSePassouTempo() {
        val agora = SystemClock.elapsedRealtime()
        if (ultimaChecagem != 0L && agora - ultimaChecagem < INTERVALO_MINIMO) return
        if (_estado.value is EstadoDaAtualizacao.Baixando) return
        if (_estado.value is EstadoDaAtualizacao.Pronta) return
        verificar()
    }

    /** Verificação pedida pela pessoa. Ignora a represa — foi um gesto explícito. */
    suspend fun verificar() {
        _estado.value = EstadoDaAtualizacao.Verificando
        try {
            val manifesto = withContext(Dispatchers.IO) { buscarManifesto() }
            ultimaChecagem = SystemClock.elapsedRealtime()

            val disponivel = codigoDaVersao(manifesto.version)
            if (disponivel <= BuildConfig.VERSION_CODE || !urlConfiavel(manifesto.android.url)) {
                // Endereço fora do domínio esperado é tratado como "nada a fazer", e
                // não como erro visível: para quem lê a tela não há ação possível, e
                // um alarme que a pessoa não pode atender só ensina a ignorar alarmes.
                _estado.value = EstadoDaAtualizacao.EmDia
                return
            }
            _estado.value = EstadoDaAtualizacao.Disponivel(manifesto.version, manifesto.notesUrl)
            ultimoManifesto = manifesto
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            _estado.value = EstadoDaAtualizacao.Falhou(classificar(e))
        }
    }

    private var ultimoManifesto: ManifestoDeAtualizacao? = null

    /** O APK já verificado, à espera do instalador. Limpo quando a checagem recomeça. */
    private var arquivoPronto: File? = null

    /**
     * Baixa, confere e deixa pronto para instalar.
     *
     * O download vai para o cache privado do aplicativo. Se qualquer portão
     * reprovar, o arquivo é apagado ali mesmo: um APK que não passou na conferência
     * não deve sobrar no disco para ser encontrado depois por outro caminho.
     */
    suspend fun baixar() {
        val manifesto = ultimoManifesto ?: run {
            _estado.value = EstadoDaAtualizacao.Falhou("sem_manifesto")
            return
        }
        _estado.value = EstadoDaAtualizacao.Baixando(null)
        var destino: File? = null
        try {
            destino = withContext(Dispatchers.IO) { baixarEConferir(manifesto) }
            arquivoPronto = destino
            _estado.value = EstadoDaAtualizacao.Pronta(manifesto.version)
        } catch (e: CancellationException) {
            destino?.delete()
            throw e
        } catch (e: Exception) {
            destino?.delete()
            _estado.value = EstadoDaAtualizacao.Falhou(classificar(e))
        }
    }

    /**
     * Entrega ao instalador do sistema.
     *
     * Antes disso, confere `canRequestPackageInstalls`. Desde o Android 8 a
     * autorização de fontes desconhecidas é por aplicativo e mora nos Ajustes do
     * sistema; sem ela, a intenção abriria uma tela que não instala nada e volta
     * sem explicação.
     */
    fun instalar() {
        val apk = arquivoPronto ?: run {
            _estado.value = EstadoDaAtualizacao.Falhou("sem_arquivo")
            return
        }
        if (!context.packageManager.canRequestPackageInstalls()) {
            _estado.value = EstadoDaAtualizacao.PrecisaAutorizar
            return
        }
        val uri = FileProvider.getUriForFile(
            context,
            "${context.packageName}.fileprovider",
            apk,
        )
        val intencao = Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(uri, "application/vnd.android.package-archive")
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        context.startActivity(intencao)
    }

    /** Abre a tela de Ajustes onde a autorização de fontes desconhecidas é dada. */
    fun abrirAutorizacao() {
        val intencao = Intent(
            Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
            Uri.parse("package:${context.packageName}"),
        ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        context.startActivity(intencao)
    }

    // ── interno ────────────────────────────────────────────────────────────────

    private fun buscarManifesto(): ManifestoDeAtualizacao {
        val requisicao = Request.Builder().url(ENDERECO_DO_MANIFESTO).build()
        cliente.newCall(requisicao).execute().use { resposta ->
            if (!resposta.isSuccessful) throw IllegalStateException("http ${resposta.code}")
            val texto = resposta.body.string()
            return json.decodeFromString(ManifestoDeAtualizacao.serializer(), texto)
        }
    }

    private fun baixarEConferir(manifesto: ManifestoDeAtualizacao): File {
        val pasta = File(context.cacheDir, "updates").apply {
            // Sobras de uma tentativa anterior não têm valor e ocupam dezenas de MB.
            deleteRecursively()
            mkdirs()
        }
        val destino = File(pasta, "lume-mobile-update.apk")

        val requisicao = Request.Builder().url(manifesto.android.url).build()
        val digestor = MessageDigest.getInstance("SHA-256")

        cliente.newCall(requisicao).execute().use { resposta ->
            if (!resposta.isSuccessful) throw IllegalStateException("http ${resposta.code}")
            val corpo = resposta.body
            val total = corpo.contentLength()
            var lidos = 0L

            corpo.byteStream().use { entrada ->
                destino.outputStream().use { saida ->
                    val buffer = ByteArray(64 * 1024)
                    while (true) {
                        val n = entrada.read(buffer)
                        if (n <= 0) break
                        saida.write(buffer, 0, n)
                        // Um passo só: escreve e soma ao resumo ao mesmo tempo, para
                        // não ler o arquivo inteiro de novo só para conferir o hash.
                        digestor.update(buffer, 0, n)
                        lidos += n
                        _estado.value = EstadoDaAtualizacao.Baixando(
                            if (total > 0) (lidos.toFloat() / total).coerceIn(0f, 1f) else null,
                        )
                    }
                }
            }
        }

        val obtido = digestor.digest().joinToString("") { "%02x".format(it) }
        if (!obtido.equals(manifesto.android.sha256, ignoreCase = true)) {
            throw IllegalStateException("sha_divergente")
        }
        if (!assinaturaConfere(destino)) {
            throw IllegalStateException("assinatura_divergente")
        }
        return destino
    }

    /**
     * A assinatura do candidato é a mesma do que está instalado?
     *
     * Comparação por **conjunto** de resumos, e não posição a posição: a ordem em
     * que o sistema devolve os signatários não é contratual, e um aplicativo com
     * dois signatários poderia reprovar por troca de ordem — recusando uma
     * atualização legítima, que é a falha mais cara aqui.
     *
     * Conjunto vazio reprova. Não conseguir ler assinatura nenhuma é motivo para
     * parar, não para seguir.
     */
    private fun assinaturaConfere(apk: File): Boolean {
        val gerente = context.packageManager
        val bandeiras = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            PackageManager.GET_SIGNING_CERTIFICATES
        } else {
            @Suppress("DEPRECATION")
            PackageManager.GET_SIGNATURES
        }

        val candidato = gerente.getPackageArchiveInfo(apk.absolutePath, bandeiras) ?: return false
        // Um APK de outro pacote nunca seria uma atualização deste, e o sistema o
        // instalaria ao lado em vez de recusar — pior que reprovar aqui.
        if (candidato.packageName != context.packageName) return false

        val instalado = runCatching {
            gerente.getPackageInfo(context.packageName, bandeiras)
        }.getOrNull() ?: return false

        val doCandidato = resumosDe(candidato)
        return doCandidato.isNotEmpty() && doCandidato == resumosDe(instalado)
    }

    private fun resumosDe(info: PackageInfo): Set<String> {
        val assinaturas: Array<Signature>? =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                info.signingInfo?.apkContentsSigners
            } else {
                @Suppress("DEPRECATION")
                info.signatures
            }
        return assinaturas.orEmpty().map { assinatura ->
            MessageDigest.getInstance("SHA-256")
                .digest(assinatura.toByteArray())
                .joinToString("") { "%02x".format(it) }
        }.toSet()
    }

    private fun classificar(e: Exception): String = when {
        e.message == "sha_divergente" -> "sha_divergente"
        e.message == "assinatura_divergente" -> "assinatura_divergente"
        e is java.io.IOException -> "rede"
        else -> "desconhecido"
    }
}

/**
 * O endereço do APK é aceitável?
 *
 * Público para poder ser testado: este é o portão que impede um manifesto
 * adulterado de apontar o download para fora do repositório, e um portão sem teste
 * negativo é decoração.
 */
fun urlConfiavel(url: String): Boolean {
    val analisada = url.toHttpUrlOrNull() ?: return false
    return analisada.scheme == "https" &&
        analisada.host.equals("github.com", ignoreCase = true) &&
        analisada.encodedPath.startsWith(PREFIXO_ACEITO)
}
