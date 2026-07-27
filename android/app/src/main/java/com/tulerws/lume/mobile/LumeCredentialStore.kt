package com.tulerws.lume.mobile

import android.content.Context
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.first
import org.json.JSONObject
import java.util.Base64

internal data class LumeStoredCredentials(
    val gateway: String,
    val desktopId: String?,
    val deviceId: String,
    val token: String,
)

private val Context.lumeCredentialDataStore by preferencesDataStore(
    name = "lume_native_credentials",
)

internal class LumeCredentialStore(
    private val context: Context,
) {
    private val cipher = LumeKeystoreCipher("lume.native.credentials.v1")

    suspend fun read(): LumeStoredCredentials? {
        val encoded = context.lumeCredentialDataStore.data.first()[CREDENTIALS_KEY] ?: return null
        val encrypted = runCatching { Base64.getDecoder().decode(encoded) }.getOrNull()
            ?: return null
        val cleartext = cipher.decrypt(encrypted)?.toString(Charsets.UTF_8) ?: return null
        return runCatching {
            val value = JSONObject(cleartext)
            LumeStoredCredentials(
                gateway = value.getString("gateway"),
                desktopId = if (value.isNull("desktopId")) {
                    null
                } else {
                    value.optString("desktopId").takeIf(String::isNotBlank)
                },
                deviceId = value.getString("deviceId"),
                token = value.getString("token"),
            ).takeIf {
                it.gateway.startsWith("http") &&
                    it.deviceId.isNotBlank() &&
                    it.token.isNotBlank()
            }
        }.getOrNull()
    }

    suspend fun save(credentials: LumeStoredCredentials) {
        val cleartext = JSONObject()
            .put("gateway", credentials.gateway)
            .put("desktopId", credentials.desktopId ?: JSONObject.NULL)
            .put("deviceId", credentials.deviceId)
            .put("token", credentials.token)
            .toString()
            .toByteArray(Charsets.UTF_8)
        val encoded = Base64.getEncoder().encodeToString(cipher.encrypt(cleartext))
        context.lumeCredentialDataStore.edit { preferences ->
            preferences[CREDENTIALS_KEY] = encoded
        }
    }

    suspend fun clear() {
        context.lumeCredentialDataStore.edit { preferences ->
            preferences.remove(CREDENTIALS_KEY)
        }
        cipher.clear()
    }

    private companion object {
        val CREDENTIALS_KEY = stringPreferencesKey("paired_device")
    }
}
