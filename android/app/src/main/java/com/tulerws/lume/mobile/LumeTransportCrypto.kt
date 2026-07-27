package com.tulerws.lume.mobile

import java.security.MessageDigest
import java.security.SecureRandom
import java.util.Base64
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

internal data class LumeEncryptedEnvelope(
    val nonce: String,
    val ciphertext: String,
)

internal object LumeTransportCrypto {
    private const val TRANSFORMATION = "AES/GCM/NoPadding"
    private val random = SecureRandom()

    fun keyFromToken(token: String): ByteArray =
        MessageDigest.getInstance("SHA-256").digest(token.toByteArray(Charsets.UTF_8))

    fun encrypt(
        key: ByteArray,
        cleartext: String,
        aad: String,
        nonce: ByteArray? = null,
    ): LumeEncryptedEnvelope {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        if (nonce == null) {
            cipher.init(Cipher.ENCRYPT_MODE, SecretKeySpec(key, "AES"))
        } else {
            cipher.init(
                Cipher.ENCRYPT_MODE,
                SecretKeySpec(key, "AES"),
                GCMParameterSpec(128, nonce),
            )
        }
        cipher.updateAAD(aad.toByteArray(Charsets.UTF_8))
        return LumeEncryptedEnvelope(
            encode(cipher.iv),
            encode(cipher.doFinal(cleartext.toByteArray(Charsets.UTF_8))),
        )
    }

    fun decrypt(key: ByteArray, envelope: LumeEncryptedEnvelope, aad: String): String {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(
            Cipher.DECRYPT_MODE,
            SecretKeySpec(key, "AES"),
            GCMParameterSpec(128, decode(envelope.nonce)),
        )
        cipher.updateAAD(aad.toByteArray(Charsets.UTF_8))
        return cipher.doFinal(decode(envelope.ciphertext)).toString(Charsets.UTF_8)
    }

    fun encode(value: ByteArray): String =
        Base64.getUrlEncoder().withoutPadding().encodeToString(value)

    private fun decode(value: String): ByteArray = Base64.getUrlDecoder().decode(value)
}
