package com.tulerws.lume.mobile

import com.getcapacitor.JSObject
import com.getcapacitor.Plugin
import com.getcapacitor.PluginCall
import com.getcapacitor.PluginMethod
import com.getcapacitor.annotation.CapacitorPlugin
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import okhttp3.Call
import okhttp3.Callback
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONObject
import java.net.InetAddress
import java.net.URI
import java.security.SecureRandom
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.TimeUnit
import kotlin.math.min

@CapacitorPlugin(name = "LumeNative")
class LumeNativePlugin : Plugin() {
    private data class ConnectionConfig(
        val gateway: String,
        val desktopId: String?,
        val deviceId: String,
        val token: String,
    )

    private val lock = Any()
    private val random = SecureRandom()
    private val scheduler = Executors.newSingleThreadScheduledExecutor()
    private val storageScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val client = OkHttpClient.Builder()
        .connectTimeout(10, TimeUnit.SECONDS)
        .readTimeout(0, TimeUnit.MILLISECONDS)
        .pingInterval(15, TimeUnit.SECONDS)
        .build()

    @Volatile
    private var desiredConfig: ConnectionConfig? = null

    @Volatile
    private var socket: WebSocket? = null

    @Volatile
    private var connectionState = "disconnected"

    @Volatile
    private var latestSnapshot: JSONObject? = null

    @Volatile
    private var appInForeground = true

    private var connectionGeneration = 0L
    private var reconnectAttempt = 0
    private var reconnectFuture: ScheduledFuture<*>? = null
    private var streamNonce: String? = null
    private var receivedSequence = 0L
    private var discoveryInProgress = false
    private var lastDiscoveryAt = 0L
    private lateinit var credentialStore: LumeCredentialStore
    private lateinit var snapshotCache: LumeSnapshotCache
    private lateinit var discovery: LumeDiscovery
    private lateinit var notifier: LumeAgentNotifier

    override fun load() {
        credentialStore = LumeCredentialStore(context.applicationContext)
        snapshotCache = LumeSnapshotCache(context.applicationContext)
        discovery = LumeDiscovery(context.applicationContext)
        notifier = LumeAgentNotifier(context.applicationContext)
        LumeRealtimeService.stop(context)
    }

    @PluginMethod
    fun connect(call: PluginCall) {
        val gateway = call.getString("gateway")?.trim()?.trimEnd('/')
        val desktopId = call.getString("desktopId")?.trim()?.takeIf(String::isNotEmpty)
        val deviceId = call.getString("deviceId")?.trim()
        val token = call.getString("token")?.trim()
        val suppliedValues = listOf(gateway, deviceId, token).count { !it.isNullOrEmpty() }
        if (suppliedValues in 1..2) {
            call.reject(
                "Gateway, deviceId and token must be supplied together.",
                "INVALID_CONNECTION",
            )
            return
        }
        storageScope.launch {
            try {
                val config = if (suppliedValues == 3) {
                    val supplied = ConnectionConfig(
                        gateway = gateway!!,
                        desktopId = desktopId,
                        deviceId = deviceId!!,
                        token = token!!,
                    )
                    validateConfig(supplied)
                    credentialStore.save(supplied.toStored())
                    supplied
                } else {
                    credentialStore.read()?.toConnectionConfig()
                        ?: throw IllegalStateException("This phone is not paired with Lume.")
                }
                restoreCachedSnapshot(config)
                if (appInForeground) {
                    LumeRealtimeService.stop(context)
                    beginConnection(config, call)
                } else {
                    prepareBackgroundConnection(config, call)
                }
            } catch (error: Exception) {
                rejectOnMain(
                    call,
                    error.message ?: "The stored Lume connection is unavailable.",
                    "CREDENTIALS_UNAVAILABLE",
                    error,
                )
            }
        }
    }

    private fun beginConnection(config: ConnectionConfig, call: PluginCall) {
        val generation: Long
        synchronized(lock) {
            if (desiredConfig == config && connectionState in setOf("connecting", "authenticating", "connected")) {
                resolveOnMain(call, statusPayload())
                return
            }
            connectionGeneration += 1
            generation = connectionGeneration
            desiredConfig = config
            reconnectAttempt = 0
            reconnectFuture?.cancel(false)
            reconnectFuture = null
            streamNonce = null
            receivedSequence = 0
            discoveryInProgress = false
            lastDiscoveryAt = 0
            socket?.cancel()
            socket = null
            connectionState = "connecting"
        }
        emitConnectionState()
        openSocket(config, generation)
        val result = statusPayload()
        result.put("credentialsStored", true)
        resolveOnMain(call, result)
    }

    private fun prepareBackgroundConnection(config: ConnectionConfig, call: PluginCall) {
        synchronized(lock) {
            connectionGeneration += 1
            desiredConfig = config
            reconnectFuture?.cancel(false)
            reconnectFuture = null
            socket?.cancel()
            socket = null
            streamNonce = null
            receivedSequence = 0
            reconnectAttempt = 0
            discoveryInProgress = false
            lastDiscoveryAt = 0
            connectionState = "background"
        }
        val backgroundEnabled =
            LumeNativePreferences.notificationsEnabled(context) &&
                LumeNativePreferences.backgroundEnabled(context)
        val serviceFailure = if (backgroundEnabled) {
            LumeRealtimeService.start(context).exceptionOrNull()
        } else {
            LumeRealtimeService.stop(context)
            null
        }
        if (serviceFailure != null) {
            disableBackgroundMonitoring()
            resumeForegroundConnection(config, allowInBackground = true)
            emitBackgroundServiceError(serviceFailure)
        } else {
            emitConnectionState("Monitoring continues in the background.")
        }
        val result = statusPayload()
        result.put("credentialsStored", true)
        resolveOnMain(call, result)
    }

    @PluginMethod
    fun disconnect(call: PluginCall) {
        disconnectInternal()
        call.resolve(statusPayload())
    }

    @PluginMethod
    fun getStatus(call: PluginCall) {
        call.resolve(statusPayload())
    }

    @PluginMethod
    fun setMonitoringPreferences(call: PluginCall) {
        val notificationsEnabled = call.getBoolean("notificationsEnabled") ?: false
        val backgroundEnabled = call.getBoolean("backgroundEnabled") ?: false
        LumeNativePreferences.update(
            context,
            notificationsEnabled = notificationsEnabled,
            backgroundEnabled = backgroundEnabled,
        )
        if (appInForeground || !notificationsEnabled || !backgroundEnabled || desiredConfig == null) {
            LumeRealtimeService.stop(context)
        }
        call.resolve(statusPayload())
    }

    @PluginMethod
    fun getStoredConnection(call: PluginCall) {
        storageScope.launch {
            try {
                val credentials = credentialStore.read()
                val result = JSObject()
                result.put("paired", credentials != null)
                if (credentials != null) {
                    result.put("gateway", credentials.gateway)
                    if (credentials.desktopId != null) {
                        result.put("desktopId", credentials.desktopId)
                    }
                    result.put("deviceId", credentials.deviceId)
                }
                resolveOnMain(call, result)
            } catch (error: Exception) {
                rejectOnMain(
                    call,
                    "The stored Lume connection could not be read.",
                    "CREDENTIALS_UNAVAILABLE",
                    error,
                )
            }
        }
    }

    @PluginMethod
    fun clearCredentials(call: PluginCall) {
        disconnectInternal()
        storageScope.launch {
            try {
                credentialStore.clear()
                snapshotCache.clear()
                notifier.clear()
                resolveOnMain(call, statusPayload())
            } catch (error: Exception) {
                rejectOnMain(
                    call,
                    "The stored Lume connection could not be cleared.",
                    "CREDENTIALS_UNAVAILABLE",
                    error,
                )
            }
        }
    }

    @PluginMethod
    fun getSnapshot(call: PluginCall) {
        val result = JSObject()
        result.put("snapshot", latestSnapshot ?: JSONObject.NULL)
        call.resolve(result)
    }

    @PluginMethod
    fun sendCommand(pluginCall: PluginCall) {
        val command = pluginCall.getObject("command")
        val config = desiredConfig
        if (command == null || config == null) {
            pluginCall.reject("Lume is not connected.", "NOT_CONNECTED")
            return
        }
        performSecureRequest(
            pluginCall,
            config,
            "POST",
            "/api/v1/commands",
            command,
        )
    }

    @PluginMethod
    fun request(pluginCall: PluginCall) {
        val config = desiredConfig
        val method = (pluginCall.getString("method", "GET") ?: "GET").uppercase()
        val path = pluginCall.getString("path") ?: ""
        val body = pluginCall.getObject("body") ?: JSONObject()
        if (config == null) {
            pluginCall.reject("Lume is not connected.", "NOT_CONNECTED")
            return
        }
        if (method !in setOf("GET", "POST") || !isAllowedApiPath(path)) {
            pluginCall.reject("The native request is not allowed.", "INVALID_REQUEST")
            return
        }
        performSecureRequest(pluginCall, config, method, path, body)
    }

    private fun performSecureRequest(
        pluginCall: PluginCall,
        config: ConnectionConfig,
        method: String,
        path: String,
        requestBody: JSONObject,
    ) {
        val requestNonce = randomValue(16)
        val payload = JSONObject()
            .put("method", method)
            .put("path", path)
            .put("body", requestBody)
            .put("timestamp", System.currentTimeMillis())
            .put("requestNonce", requestNonce)
        val encrypted = encrypt(
            transportKey(config.token),
            payload.toString(),
            "lume-secure-request-v1",
        )
        val body = JSONObject()
            .put("deviceId", config.deviceId)
            .put("nonce", encrypted.nonce)
            .put("ciphertext", encrypted.ciphertext)
            .toString()
            .toRequestBody(JSON_MEDIA_TYPE)
        val request = Request.Builder()
            .url("${config.gateway}/api/v1/secure")
            .post(body)
            .build()
        client.newCall(request).enqueue(object : Callback {
            override fun onFailure(call: Call, e: java.io.IOException) {
                rejectOnMain(pluginCall, e.message ?: "The request could not be sent.", "REQUEST_FAILED", e)
            }

            override fun onResponse(call: Call, response: Response) {
                response.use {
                    try {
                        val outer = JSONObject(response.body.string())
                        if (!response.isSuccessful) {
                            val message = outer.optJSONObject("error")?.optString("message")
                                ?: "Lume returned ${response.code}."
                            rejectOnMain(pluginCall, message, "REQUEST_FAILED")
                            return
                        }
                        val decoded = decrypt(
                            transportKey(config.token),
                            LumeEncryptedEnvelope(
                                outer.getString("nonce"),
                                outer.getString("ciphertext"),
                            ),
                            "lume-secure-response-v1:$requestNonce",
                        )
                        val secureResponse = JSONObject(decoded)
                        val status = secureResponse.optInt("status", 500)
                        val responseBody = secureResponse.optJSONObject("body") ?: JSONObject()
                        if (status !in 200..299) {
                            if (status == 401) invalidateCredentials()
                            val message = responseBody.optJSONObject("error")?.optString("message")
                                ?: "Lume returned $status."
                            rejectOnMain(pluginCall, message, "REQUEST_FAILED")
                            return
                        }
                        resolveOnMain(pluginCall, JSObject(responseBody.toString()))
                    } catch (error: Exception) {
                        rejectOnMain(pluginCall, "The Lume response was invalid.", "REQUEST_FAILED", error)
                    }
                }
            }
        })
    }

    override fun handleOnResume() {
        super.handleOnResume()
        appInForeground = true
        LumeRealtimeService.stop(context)
        desiredConfig?.let(::resumeForegroundConnection)
    }

    override fun handleOnPause() {
        appInForeground = false
        handoffToBackground()
        super.handleOnPause()
    }

    override fun handleOnDestroy() {
        appInForeground = false
        handoffToBackground()
        disconnectInternal(stopBackgroundService = false)
        storageScope.cancel()
        snapshotCache.close()
        scheduler.shutdownNow()
        client.dispatcher.executorService.shutdown()
        client.connectionPool.evictAll()
        super.handleOnDestroy()
    }

    private fun handoffToBackground() {
        val config = suspendForegroundConnection() ?: return
        if (
            !LumeNativePreferences.notificationsEnabled(context) ||
            !LumeNativePreferences.backgroundEnabled(context)
        ) {
            return
        }
        LumeRealtimeService.start(context).onFailure { error ->
            disableBackgroundMonitoring()
            if (!appInForeground) {
                resumeForegroundConnection(config, allowInBackground = true)
            }
            emitBackgroundServiceError(error)
        }
    }

    private fun emitBackgroundServiceError(error: Throwable) {
        emit(
            "streamError",
            JSONObject()
                .put("code", "background_service_unavailable")
                .put(
                    "message",
                    error.message ?: "Android could not start background monitoring.",
                )
                .put("retryable", true),
        )
    }

    private fun disableBackgroundMonitoring() {
        LumeNativePreferences.update(
            context,
            notificationsEnabled = LumeNativePreferences.notificationsEnabled(context),
            backgroundEnabled = false,
        )
    }

    private fun suspendForegroundConnection(): ConnectionConfig? {
        val config = synchronized(lock) {
            val current = desiredConfig ?: return null
            connectionGeneration += 1
            reconnectFuture?.cancel(false)
            reconnectFuture = null
            socket?.close(1000, "Lume Mobile moved to background")
            socket = null
            streamNonce = null
            receivedSequence = 0
            reconnectAttempt = 0
            discoveryInProgress = false
            lastDiscoveryAt = 0
            connectionState = "background"
            current
        }
        emitConnectionState("Monitoring continues in the background.")
        return config
    }

    private fun resumeForegroundConnection(
        config: ConnectionConfig,
        allowInBackground: Boolean = false,
    ) {
        val generation: Long
        synchronized(lock) {
            if (
                (!allowInBackground && !appInForeground) ||
                desiredConfig != config ||
                connectionState in setOf("connecting", "authenticating", "connected")
            ) {
                return
            }
            connectionGeneration += 1
            generation = connectionGeneration
            reconnectAttempt = 0
            reconnectFuture?.cancel(false)
            reconnectFuture = null
            streamNonce = null
            receivedSequence = 0
            discoveryInProgress = false
            lastDiscoveryAt = 0
            socket?.cancel()
            socket = null
            connectionState = "connecting"
        }
        emitConnectionState()
        openSocket(config, generation)
    }

    private fun openSocket(config: ConnectionConfig, generation: Long) {
        if (!isCurrent(config, generation)) return
        setConnectionState("connecting")
        try {
            val request = Request.Builder()
                .url(realtimeUrl(config.gateway))
                .header("Sec-WebSocket-Protocol", REALTIME_SUBPROTOCOL)
                .build()
            socket = client.newWebSocket(request, socketListener(config, generation))
        } catch (error: Exception) {
            scheduleReconnect(config, generation, error.message)
        }
    }

    private fun socketListener(
        config: ConnectionConfig,
        generation: Long,
    ) = object : WebSocketListener() {
        override fun onOpen(webSocket: WebSocket, response: Response) {
            if (!isCurrent(config, generation)) {
                webSocket.cancel()
                return
            }
            setConnectionState("authenticating")
            val activeStreamNonce = randomValue(18)
            val requestNonce = randomValue(16)
            synchronized(lock) {
                streamNonce = activeStreamNonce
                receivedSequence = 0
            }
            val payload = JSONObject()
                .put("timestamp", System.currentTimeMillis())
                .put("requestNonce", requestNonce)
                .put("streamNonce", activeStreamNonce)
            val encrypted = encrypt(
                transportKey(config.token),
                payload.toString(),
                "lume-stream-auth-v1",
            )
            val authentication = JSONObject()
                .put("deviceId", config.deviceId)
                .put("nonce", encrypted.nonce)
                .put("ciphertext", encrypted.ciphertext)
            if (!webSocket.send(authentication.toString())) {
                scheduleReconnect(config, generation, "Authentication could not be sent.")
            }
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            if (!isCurrent(config, generation)) return
            try {
                handleStreamMessage(config, text)
            } catch (error: Exception) {
                emit(
                    "streamError",
                    JSONObject()
                        .put("code", "invalid_stream_message")
                        .put("message", error.message ?: "The realtime message was invalid.")
                        .put("retryable", true),
                )
            }
        }

        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
            if (code == 1008 && reason == "authentication_failed") {
                invalidateCredentials(config, generation)
            }
            webSocket.close(1000, null)
        }

        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
            scheduleReconnect(config, generation, reason.ifBlank { null })
        }

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            if (response?.code == 401 || response?.code == 403) {
                invalidateCredentials(config, generation)
            } else {
                scheduleReconnect(config, generation, t.message)
            }
        }
    }

    private fun handleStreamMessage(config: ConnectionConfig, text: String) {
        val outer = JSONObject(text)
        if (outer.optString("type") != "secure_message") return
        val sequence = outer.getLong("sequence")
        val activeNonce: String
        synchronized(lock) {
            if (sequence <= receivedSequence) return
            activeNonce = streamNonce ?: return
        }
        val decoded = decrypt(
            transportKey(config.token),
            LumeEncryptedEnvelope(
                outer.getString("nonce"),
                outer.getString("ciphertext"),
            ),
            "lume-stream-message-v1:$activeNonce:$sequence",
        )
        val message = JSONObject(decoded)
        synchronized(lock) {
            if (sequence <= receivedSequence) return
            receivedSequence = sequence
        }
        when (message.optString("type")) {
            "hello" -> {
                synchronized(lock) {
                    reconnectAttempt = 0
                    reconnectFuture?.cancel(false)
                    reconnectFuture = null
                    discoveryInProgress = false
                }
                setConnectionState("connected")
            }

            "snapshot" -> {
                val snapshot = message.getJSONObject("snapshot")
                latestSnapshot = snapshot
                notifier.observe(snapshot)
                persistSnapshot(config.deviceId, snapshot)
                emit(
                    "sessionSnapshot",
                    JSONObject()
                        .put("sequence", message.optLong("sequence"))
                        .put("cached", false)
                        .put("snapshot", snapshot),
                )
            }

            "update" -> {
                val snapshot = message.getJSONObject("snapshot")
                latestSnapshot = snapshot
                notifier.observe(snapshot)
                persistSnapshot(config.deviceId, snapshot)
                emit(
                    "sessionDelta",
                    JSONObject()
                        .put("events", message.optJSONArray("events"))
                        .put("cached", false)
                        .put("snapshot", snapshot),
                )
            }

            "error" -> emit("streamError", message)
        }
    }

    private fun scheduleReconnect(
        config: ConnectionConfig,
        generation: Long,
        reason: String?,
    ) {
        val delay: Long
        val attempt: Int
        val shouldDiscover: Boolean
        synchronized(lock) {
            if (!isCurrentLocked(config, generation)) return
            if (reconnectFuture?.isDone == false) return
            socket = null
            reconnectAttempt += 1
            attempt = reconnectAttempt
            connectionState = "reconnecting"
            val now = System.currentTimeMillis()
            shouldDiscover = attempt >= 2 &&
                config.desktopId != null &&
                !discoveryInProgress &&
                now - lastDiscoveryAt >= DISCOVERY_INTERVAL_MS
            if (shouldDiscover) {
                discoveryInProgress = true
                lastDiscoveryAt = now
                reconnectFuture = null
                delay = 0
            } else {
                val multiplier = 1L shl min(attempt - 1, 5)
                delay = min(15_000L, 500L * multiplier)
                reconnectFuture = scheduler.schedule(
                    { openSocket(config, generation) },
                    delay,
                    TimeUnit.MILLISECONDS,
                )
            }
        }
        emitConnectionState(
            if (shouldDiscover) "Looking for this Lume desktop on the local network." else reason,
            attempt,
            delay.takeUnless { shouldDiscover },
        )
        if (shouldDiscover) discoverGateway(config, generation)
    }

    private fun discoverGateway(config: ConnectionConfig, generation: Long) {
        storageScope.launch {
            val discoveredGateway = runCatching {
                discovery.findGateway(config.desktopId!!)
            }.getOrNull()
            val candidate = discoveredGateway
                ?.takeIf { it != config.gateway }
                ?.let { config.copy(gateway = it) }
            val authenticated = candidate != null &&
                runCatching { probeGateway(candidate) }.getOrDefault(false)
            if (authenticated) {
                val verifiedCandidate = checkNotNull(candidate)
                val nextGeneration: Long
                synchronized(lock) {
                    if (!isCurrentLocked(config, generation)) {
                        discoveryInProgress = false
                        return@launch
                    }
                    connectionGeneration += 1
                    nextGeneration = connectionGeneration
                    desiredConfig = verifiedCandidate
                    reconnectAttempt = 0
                    reconnectFuture?.cancel(false)
                    reconnectFuture = null
                    discoveryInProgress = false
                    streamNonce = null
                    receivedSequence = 0
                    connectionState = "connecting"
                }
                runCatching { credentialStore.save(verifiedCandidate.toStored()) }
                emitConnectionState("Lume desktop found on the local network.")
                openSocket(verifiedCandidate, nextGeneration)
                return@launch
            }

            synchronized(lock) {
                if (!isCurrentLocked(config, generation)) {
                    discoveryInProgress = false
                    return@launch
                }
                discoveryInProgress = false
            }
            scheduleReconnect(
                config,
                generation,
                "The paired Lume desktop was not found on the local network.",
            )
        }
    }

    private fun probeGateway(config: ConnectionConfig): Boolean {
        return LumeGatewayProbe.authenticate(client, config.toStored(), config.gateway)
    }

    private fun disconnectInternal(stopBackgroundService: Boolean = true) {
        synchronized(lock) {
            connectionGeneration += 1
            desiredConfig = null
            reconnectFuture?.cancel(false)
            reconnectFuture = null
            socket?.close(1000, "Lume Mobile disconnected")
            socket = null
            streamNonce = null
            receivedSequence = 0
            reconnectAttempt = 0
            discoveryInProgress = false
            lastDiscoveryAt = 0
            connectionState = "disconnected"
        }
        if (stopBackgroundService) LumeRealtimeService.stop(context)
        emitConnectionState()
    }

    private fun invalidateCredentials(
        config: ConnectionConfig? = desiredConfig,
        generation: Long = connectionGeneration,
    ) {
        if (config != null && !isCurrent(config, generation)) return
        disconnectInternal()
        storageScope.launch {
            runCatching { credentialStore.clear() }
            runCatching { snapshotCache.clear() }
            runCatching { notifier.clear() }
        }
        emit(
            "streamError",
            JSONObject()
                .put("code", "authentication_failed")
                .put("message", "Pair this phone with Lume again.")
                .put("retryable", false),
        )
    }

    private fun setConnectionState(value: String) {
        synchronized(lock) {
            connectionState = value
        }
        emitConnectionState()
    }

    private suspend fun restoreCachedSnapshot(config: ConnectionConfig) {
        val cached = snapshotCache.read(config.deviceId) ?: return
        val snapshot = runCatching { JSONObject(cached.json) }.getOrNull() ?: return
        latestSnapshot = snapshot
        emit(
            "sessionSnapshot",
            JSONObject()
                .put("sequence", 0)
                .put("cached", true)
                .put("cachedAt", cached.updatedAt)
                .put("snapshot", snapshot),
        )
    }

    private fun persistSnapshot(deviceId: String, snapshot: JSONObject) {
        val serialized = snapshot.toString()
        storageScope.launch {
            runCatching { snapshotCache.save(deviceId, serialized) }
        }
    }

    private fun statusPayload(): JSObject {
        val result = JSObject()
        result.put("available", true)
        result.put("status", connectionState)
        result.put("reconnectAttempt", reconnectAttempt)
        result.put(
            "notificationsEnabled",
            LumeNativePreferences.notificationsEnabled(context),
        )
        result.put(
            "backgroundEnabled",
            LumeNativePreferences.backgroundEnabled(context),
        )
        return result
    }

    private fun emitConnectionState(
        reason: String? = null,
        attempt: Int = reconnectAttempt,
        retryInMs: Long? = null,
    ) {
        val payload = JSONObject()
            .put("available", true)
            .put("status", connectionState)
            .put("reconnectAttempt", attempt)
        if (!reason.isNullOrBlank()) payload.put("reason", reason)
        if (retryInMs != null) payload.put("retryInMs", retryInMs)
        emit("connectionChanged", payload)
    }

    private fun emit(event: String, payload: JSONObject) {
        val data = JSObject(payload.toString())
        bridge.executeOnMainThread { notifyListeners(event, data) }
    }

    private fun resolveOnMain(call: PluginCall, value: JSObject) {
        bridge.executeOnMainThread { call.resolve(value) }
    }

    private fun rejectOnMain(
        call: PluginCall,
        message: String,
        code: String,
        error: Exception? = null,
    ) {
        bridge.executeOnMainThread {
            if (error == null) call.reject(message, code) else call.reject(message, code, error)
        }
    }

    private fun isCurrent(config: ConnectionConfig, generation: Long): Boolean =
        synchronized(lock) { isCurrentLocked(config, generation) }

    private fun isCurrentLocked(config: ConnectionConfig, generation: Long): Boolean =
        desiredConfig == config && connectionGeneration == generation

    private fun validateConfig(config: ConnectionConfig) {
        val uri = runCatching { URI(config.gateway) }
            .getOrElse { throw IllegalArgumentException("The gateway address is invalid.") }
        if (
            uri.scheme?.lowercase() !in setOf("http", "https") ||
            uri.host.isNullOrBlank() ||
            (uri.scheme.equals("http", ignoreCase = true) && !isLocalNetworkHost(uri.host)) ||
            (config.desktopId != null && config.desktopId.length !in 8..128) ||
            config.deviceId.isBlank() ||
            config.deviceId.length > 128 ||
            config.token.isBlank() ||
            config.token.length > 256
        ) {
            throw IllegalArgumentException("The stored Lume connection is invalid.")
        }
    }

    private fun isLocalNetworkHost(host: String): Boolean {
        val normalized = host.trim().trim('[', ']').lowercase()
        if (normalized == "localhost" || normalized.endsWith(".local")) return true
        return runCatching {
            val addresses = InetAddress.getAllByName(normalized)
            addresses.isNotEmpty() && addresses.all(::isLocalNetworkAddress)
        }.getOrDefault(false)
    }

    private fun isLocalNetworkAddress(address: InetAddress): Boolean {
        if (address.isLoopbackAddress || address.isSiteLocalAddress || address.isLinkLocalAddress) {
            return true
        }
        val bytes = address.address
        return bytes.size == 16 && (bytes[0].toInt() and 0xfe) == 0xfc
    }

    private fun ConnectionConfig.toStored() = LumeStoredCredentials(
        gateway = gateway,
        desktopId = desktopId,
        deviceId = deviceId,
        token = token,
    )

    private fun LumeStoredCredentials.toConnectionConfig() = ConnectionConfig(
        gateway = gateway,
        desktopId = desktopId,
        deviceId = deviceId,
        token = token,
    ).also(::validateConfig)

    private fun isAllowedApiPath(path: String): Boolean {
        if (!path.startsWith("/api/v1/")) return false
        return path.substringBefore('?') !in setOf(
            "/api/v1/pair",
            "/api/v1/pair-secure",
            "/api/v1/secure",
            REALTIME_PATH,
        )
    }

    private fun realtimeUrl(gateway: String): String {
        val uri = URI(gateway)
        val scheme = when (uri.scheme.lowercase()) {
            "http" -> "ws"
            "https" -> "wss"
            else -> throw IllegalArgumentException("Unsupported gateway scheme.")
        }
        return URI(scheme, uri.userInfo, uri.host, uri.port, REALTIME_PATH, null, null).toString()
    }

    private fun transportKey(token: String): ByteArray =
        LumeTransportCrypto.keyFromToken(token)

    private fun encrypt(key: ByteArray, cleartext: String, aad: String): LumeEncryptedEnvelope =
        LumeTransportCrypto.encrypt(key, cleartext, aad)

    private fun decrypt(key: ByteArray, envelope: LumeEncryptedEnvelope, aad: String): String =
        LumeTransportCrypto.decrypt(key, envelope, aad)

    private fun randomValue(bytes: Int): String {
        val value = ByteArray(bytes)
        random.nextBytes(value)
        return LumeTransportCrypto.encode(value)
    }

    private companion object {
        const val REALTIME_PATH = "/api/v1/ws"
        const val REALTIME_SUBPROTOCOL = "lume.hub.v1"
        const val DISCOVERY_INTERVAL_MS = 15_000L
        val JSON_MEDIA_TYPE = "application/json; charset=utf-8".toMediaType()
    }
}
