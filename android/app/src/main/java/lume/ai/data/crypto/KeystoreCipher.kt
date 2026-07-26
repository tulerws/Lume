package lume.ai.data.crypto

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Cifra um blob com uma chave que nunca sai do aparelho.
 *
 * ## Por que não `EncryptedSharedPreferences`
 *
 * Ela foi descontinuada em `security-crypto:1.1.0-alpha07`, e tem histórico de
 * corrupção de keyset em alguns fabricantes — que se manifesta, para o usuário,
 * como "o aplicativo perdeu o pareamento sozinho". A chave aqui é gerada dentro
 * do Keystore, tem respaldo de hardware quando o aparelho oferece, e **não é
 * exportável**: nem este código consegue lê-la.
 *
 * ## AES/GCM, e o IV junto do texto cifrado
 *
 * GCM autentica além de cifrar: um blob adulterado falha na decifragem em vez de
 * devolver bytes plausíveis. O IV tem 12 bytes, é gerado pelo próprio `Cipher` a
 * cada escrita, e vai **na frente** do resultado — precisa ser guardado, não é
 * segredo, e reusá-lo com a mesma chave quebraria o GCM inteiro.
 *
 * ## Sem exigência de autenticação do usuário
 *
 * `setUserAuthenticationRequired` fica em `false` de propósito. O bloqueio do
 * aplicativo é assunto do `BiometricPrompt`, na abertura, e amarrar a chave à
 * biometria impediria a reconexão em segundo plano — que é justamente o que a v2
 * vai precisar fazer.
 */
@Singleton
class KeystoreCipher @Inject constructor() {

    private val keystore = KeyStore.getInstance(PROVEDOR).apply { load(null) }

    fun cifrar(claro: ByteArray): ByteArray {
        val cifra = Cipher.getInstance(TRANSFORMACAO)
        cifra.init(Cipher.ENCRYPT_MODE, chave())
        val cifrado = cifra.doFinal(claro)
        return cifra.iv + cifrado
    }

    /**
     * @return `null` quando o blob não decifra.
     *
     * Acontece de verdade em dois casos, e nos dois a resposta certa é a mesma —
     * tratar como "não há credencial" e voltar ao pareamento: o usuário
     * acrescentou bloqueio de tela num aparelho que não tinha (o Android invalida
     * chaves nesse evento em algumas versões), ou o blob foi adulterado.
     */
    fun decifrar(blob: ByteArray): ByteArray? {
        if (blob.size <= TAMANHO_DO_IV) return null
        return runCatching {
            val cifra = Cipher.getInstance(TRANSFORMACAO)
            cifra.init(
                Cipher.DECRYPT_MODE,
                chave(),
                GCMParameterSpec(TAMANHO_DA_TAG, blob, 0, TAMANHO_DO_IV),
            )
            cifra.doFinal(blob, TAMANHO_DO_IV, blob.size - TAMANHO_DO_IV)
        }.getOrNull()
    }

    /** Apaga a chave. O blob que sobrar vira ruído indecifrável, que é o objetivo. */
    fun esquecer() {
        runCatching { keystore.deleteEntry(ALIAS) }
    }

    private fun chave(): SecretKey {
        (keystore.getEntry(ALIAS, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }
        val gerador = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, PROVEDOR)
        gerador.init(
            KeyGenParameterSpec.Builder(
                ALIAS,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .setUserAuthenticationRequired(false)
                .build(),
        )
        return gerador.generateKey()
    }

    private companion object {
        const val PROVEDOR = "AndroidKeyStore"
        const val TRANSFORMACAO = "AES/GCM/NoPadding"

        /** Alias dedicado: nada mais neste aplicativo usa esta chave. */
        const val ALIAS = "lume.credencial.v1"
        const val TAMANHO_DO_IV = 12
        const val TAMANHO_DA_TAG = 128
    }
}
