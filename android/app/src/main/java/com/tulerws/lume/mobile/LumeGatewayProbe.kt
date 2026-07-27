package com.tulerws.lume.mobile

import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import java.security.SecureRandom
import java.util.concurrent.TimeUnit

internal object LumeGatewayProbe {
    private val random = SecureRandom()
    private val jsonMediaType = "application/json; charset=utf-8".toMediaType()

    fun authenticate(
        client: OkHttpClient,
        credentials: LumeStoredCredentials,
        gateway: String,
    ): Boolean {
        val requestNonce = randomValue(16)
        val payload = JSONObject()
            .put("method", "GET")
            .put("path", "/api/v1/me")
            .put("body", JSONObject())
            .put("timestamp", System.currentTimeMillis())
            .put("requestNonce", requestNonce)
        val key = LumeTransportCrypto.keyFromToken(credentials.token)
        val encrypted = LumeTransportCrypto.encrypt(
            key,
            payload.toString(),
            "lume-secure-request-v1",
        )
        val body = JSONObject()
            .put("deviceId", credentials.deviceId)
            .put("nonce", encrypted.nonce)
            .put("ciphertext", encrypted.ciphertext)
            .toString()
            .toRequestBody(jsonMediaType)
        val request = Request.Builder()
            .url("${gateway.trimEnd('/')}/api/v1/secure")
            .post(body)
            .build()
        val call = client.newCall(request)
        call.timeout().timeout(PROBE_TIMEOUT_MS, TimeUnit.MILLISECONDS)
        return call.execute().use { response ->
            if (!response.isSuccessful) return@use false
            val outer = JSONObject(response.body.string())
            val decoded = LumeTransportCrypto.decrypt(
                key,
                LumeEncryptedEnvelope(
                    outer.getString("nonce"),
                    outer.getString("ciphertext"),
                ),
                "lume-secure-response-v1:$requestNonce",
            )
            val secureResponse = JSONObject(decoded)
            secureResponse.optInt("status", 500) == 200 &&
                secureResponse.optJSONObject("body")
                    ?.optString("id") == credentials.deviceId
        }
    }

    private fun randomValue(bytes: Int): String {
        val value = ByteArray(bytes)
        random.nextBytes(value)
        return LumeTransportCrypto.encode(value)
    }

    private const val PROBE_TIMEOUT_MS = 8_000L
}
