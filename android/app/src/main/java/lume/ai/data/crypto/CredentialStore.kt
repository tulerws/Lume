package lume.ai.data.crypto

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.first
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import java.util.Base64
import javax.inject.Inject
import javax.inject.Singleton

/**
 * A credencial do aparelho, como ela é guardada.
 *
 * Os campos são os que `docs/ANDROID.md` lista: identificador, token, fingerprint,
 * candidatos, porta e nome do desktop. Tudo que é preciso para reconectar sem
 * ler o QR de novo.
 *
 * O [token] trafega **uma única vez**, no `pair.accepted`. Depois disso o desktop
 * guarda só o `SHA-256` dele e não tem como reconstruí-lo — perder o token aqui
 * significa parear de novo, não recuperá-lo. Por isso ele é gravado antes de
 * qualquer outra coisa acontecer na conexão.
 */
@Serializable
data class CredencialGuardada(
    val deviceId: String,
    val token: String,
    /** Hexadecimal: JSON não carrega bytes, e hex sobrevive a inspeção humana. */
    val fingerprintEmHex: String,
    val candidatos: List<String>,
    val porta: Int,
    val nomeDoDesktop: String,
    /** Rótulo já formatado para a tela de Ajustes: "12 jul". */
    val pareadoEm: String,
)

private val Context.armazenamento: DataStore<Preferences> by preferencesDataStore("lume_credencial")

/**
 * Onde a credencial vive.
 *
 * Um único blob, cifrado pelo [KeystoreCipher] e guardado em base64 no DataStore.
 * Não é o DataStore que protege — o armazenamento do aplicativo já é isolado por
 * UID e cifrado pelo FBE do Android. A cifra existe para o caso de o arquivo
 * escapar do sandbox: backup mal configurado, extração por `adb` num aparelho com
 * depuração ligada, imagem de disco.
 *
 * Blob único, e não um campo por chave, por dois motivos: cifrar seis vezes não
 * protege mais do que cifrar uma, e um campo em claro esquecido por engano numa
 * adição futura seria invisível na revisão.
 */
@Singleton
class CredentialStore @Inject constructor(
    @param:ApplicationContext private val contexto: Context,
    private val cifra: KeystoreCipher,
) {

    suspend fun ler(): CredencialGuardada? {
        val emBase64 = contexto.armazenamento.data.first()[CHAVE] ?: return null
        val blob = runCatching { Base64.getDecoder().decode(emBase64) }.getOrNull() ?: return null
        val claro = cifra.decifrar(blob) ?: return null
        return runCatching { Json.decodeFromString<CredencialGuardada>(claro.decodeToString()) }
            .getOrNull()
    }

    suspend fun gravar(credencial: CredencialGuardada) {
        val blob = cifra.cifrar(Json.encodeToString(credencial).encodeToByteArray())
        contexto.armazenamento.edit { it[CHAVE] = Base64.getEncoder().encodeToString(blob) }
    }

    /**
     * Esquece o aparelho.
     *
     * Apaga o blob **e** a chave. Só apagar o blob deixaria a chave órfã no
     * Keystore; só apagar a chave deixaria um blob que ninguém decifra — os dois
     * juntos é o que faz "esquecer" significar esquecer.
     */
    suspend fun apagar() {
        contexto.armazenamento.edit { it.remove(CHAVE) }
        cifra.esquecer()
    }

    private companion object {
        val CHAVE = stringPreferencesKey("credencial")
    }
}
