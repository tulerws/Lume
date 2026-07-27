package com.tulerws.lume.mobile

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONObject
import java.net.URI
import java.security.SecureRandom
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.TimeUnit
import kotlin.math.min

internal class LumeRealtimeService : Service() {
    private val lock = Any()
    private val random = SecureRandom()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val scheduler = Executors.newSingleThreadScheduledExecutor()
    private val client = OkHttpClient.Builder()
        .connectTimeout(10, TimeUnit.SECONDS)
        .readTimeout(0, TimeUnit.MILLISECONDS)
        .pingInterval(15, TimeUnit.SECONDS)
        .build()
    private lateinit var credentialStore: LumeCredentialStore
    private lateinit var snapshotCache: LumeSnapshotCache
    private lateinit var discovery: LumeDiscovery
    private lateinit var notifier: LumeAgentNotifier
    private var credentials: LumeStoredCredentials? = null
    private var socket: WebSocket? = null
    private var reconnectFuture: ScheduledFuture<*>? = null
    private var generation = 0L
    private var reconnectAttempt = 0
    private var streamNonce: String? = null
    private var receivedSequence = 0L
    private var discoveryInProgress = false
    private var lastDiscoveryAt = 0L

    override fun onCreate() {
        super.onCreate()
        credentialStore = LumeCredentialStore(applicationContext)
        snapshotCache = LumeSnapshotCache(applicationContext)
        discovery = LumeDiscovery(applicationContext)
        notifier = LumeAgentNotifier(applicationContext)
        startForeground(NOTIFICATION_ID, serviceNotification("Starting secure monitoring…"))
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (!LumeNativePreferences.backgroundEnabled(applicationContext)) {
            stopSelf()
            return START_NOT_STICKY
        }
        scope.launch {
            val stored = credentialStore.read()
            if (stored == null) {
                stopSelf()
            } else {
                beginConnection(stored)
            }
        }
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        synchronized(lock) {
            generation += 1
            reconnectFuture?.cancel(false)
            reconnectFuture = null
            socket?.cancel()
            socket = null
        }
        scope.cancel()
        scheduler.shutdownNow()
        snapshotCache.close()
        client.dispatcher.executorService.shutdown()
        client.connectionPool.evictAll()
        super.onDestroy()
    }

    private fun beginConnection(next: LumeStoredCredentials) {
        val nextGeneration: Long
        synchronized(lock) {
            if (credentials == next && socket != null) return
            generation += 1
            nextGeneration = generation
            credentials = next
            reconnectAttempt = 0
            reconnectFuture?.cancel(false)
            reconnectFuture = null
            socket?.cancel()
            socket = null
            streamNonce = null
            receivedSequence = 0
            discoveryInProgress = false
        }
        updateForeground("Connecting to Lume…")
        openSocket(next, nextGeneration)
    }

    private fun openSocket(config: LumeStoredCredentials, activeGeneration: Long) {
        if (!isCurrent(config, activeGeneration)) return
        try {
            val request = Request.Builder()
                .url(realtimeUrl(config.gateway))
                .header("Sec-WebSocket-Protocol", REALTIME_SUBPROTOCOL)
                .build()
            socket = client.newWebSocket(request, socketListener(config, activeGeneration))
        } catch (error: Exception) {
            scheduleReconnect(config, activeGeneration)
        }
    }

    private fun socketListener(
        config: LumeStoredCredentials,
        activeGeneration: Long,
    ) = object : WebSocketListener() {
        override fun onOpen(webSocket: WebSocket, response: Response) {
            if (!isCurrent(config, activeGeneration)) {
                webSocket.cancel()
                return
            }
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
            val encrypted = LumeTransportCrypto.encrypt(
                LumeTransportCrypto.keyFromToken(config.token),
                payload.toString(),
                "lume-stream-auth-v1",
            )
            val authentication = JSONObject()
                .put("deviceId", config.deviceId)
                .put("nonce", encrypted.nonce)
                .put("ciphertext", encrypted.ciphertext)
            if (!webSocket.send(authentication.toString())) {
                scheduleReconnect(config, activeGeneration)
            }
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            if (!isCurrent(config, activeGeneration)) return
            runCatching { handleStreamMessage(config, text) }
        }

        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
            if (code == 1008 && reason == "authentication_failed") {
                updateForeground("Pair this phone with Lume again.")
                stopSelf()
            }
            webSocket.close(1000, null)
        }

        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
            scheduleReconnect(config, activeGeneration)
        }

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            if (response?.code == 401 || response?.code == 403) {
                updateForeground("Pair this phone with Lume again.")
                stopSelf()
            } else {
                scheduleReconnect(config, activeGeneration)
            }
        }
    }

    private fun handleStreamMessage(config: LumeStoredCredentials, text: String) {
        val outer = JSONObject(text)
        if (outer.optString("type") != "secure_message") return
        val sequence = outer.getLong("sequence")
        val activeNonce = synchronized(lock) {
            if (sequence <= receivedSequence) return
            streamNonce ?: return
        }
        val decoded = LumeTransportCrypto.decrypt(
            LumeTransportCrypto.keyFromToken(config.token),
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
                updateForeground("Monitoring your Lume agents")
            }

            "snapshot", "update" -> {
                val snapshot = message.getJSONObject("snapshot")
                notifier.observe(snapshot)
                scope.launch {
                    runCatching { snapshotCache.save(config.deviceId, snapshot.toString()) }
                }
            }
        }
    }

    private fun scheduleReconnect(
        config: LumeStoredCredentials,
        activeGeneration: Long,
    ) {
        val shouldDiscover: Boolean
        synchronized(lock) {
            if (!isCurrentLocked(config, activeGeneration)) return
            if (reconnectFuture?.isDone == false || discoveryInProgress) return
            socket = null
            reconnectAttempt += 1
            val now = System.currentTimeMillis()
            shouldDiscover = reconnectAttempt >= 2 &&
                config.desktopId != null &&
                now - lastDiscoveryAt >= DISCOVERY_INTERVAL_MS
            if (shouldDiscover) {
                discoveryInProgress = true
                lastDiscoveryAt = now
            } else {
                val multiplier = 1L shl min(reconnectAttempt - 1, 5)
                val delay = min(15_000L, 500L * multiplier)
                reconnectFuture = scheduler.schedule(
                    { openSocket(config, activeGeneration) },
                    delay,
                    TimeUnit.MILLISECONDS,
                )
            }
        }
        updateForeground(
            if (shouldDiscover) "Finding this Lume desktop…" else "Reconnecting to Lume…",
        )
        if (shouldDiscover) discoverGateway(config, activeGeneration)
    }

    private fun discoverGateway(
        config: LumeStoredCredentials,
        activeGeneration: Long,
    ) {
        scope.launch {
            val candidateGateway = runCatching {
                discovery.findGateway(config.desktopId!!)
            }.getOrNull()
            val candidate = candidateGateway
                ?.takeIf { it != config.gateway }
                ?.let { config.copy(gateway = it) }
            val authenticated = candidate != null && runCatching {
                LumeGatewayProbe.authenticate(client, candidate, candidate.gateway)
            }.getOrDefault(false)
            if (authenticated) {
                val verified = checkNotNull(candidate)
                runCatching { credentialStore.save(verified) }
                beginConnection(verified)
                return@launch
            }
            synchronized(lock) {
                if (!isCurrentLocked(config, activeGeneration)) return@launch
                discoveryInProgress = false
            }
            scheduleReconnect(config, activeGeneration)
        }
    }

    private fun isCurrent(config: LumeStoredCredentials, activeGeneration: Long): Boolean =
        synchronized(lock) { isCurrentLocked(config, activeGeneration) }

    private fun isCurrentLocked(
        config: LumeStoredCredentials,
        activeGeneration: Long,
    ): Boolean = credentials == config && generation == activeGeneration

    private fun updateForeground(detail: String) {
        val manager = getSystemService(NotificationManager::class.java)
        manager.notify(NOTIFICATION_ID, serviceNotification(detail))
    }

    private fun serviceNotification(detail: String): Notification {
        ensureServiceChannel()
        val intent = Intent(this, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        val pendingIntent = PendingIntent.getActivity(
            this,
            NOTIFICATION_ID,
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Builder(this, SERVICE_CHANNEL_ID)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle("Lume background monitoring")
            .setContentText(detail)
            .setContentIntent(pendingIntent)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()
    }

    private fun ensureServiceChannel() {
        val manager = getSystemService(NotificationManager::class.java)
        val channel = NotificationChannel(
            SERVICE_CHANNEL_ID,
            "Background monitoring",
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = "Keeps the secure local connection active when Lume is closed"
            setShowBadge(false)
        }
        manager.createNotificationChannel(channel)
    }

    private fun realtimeUrl(gateway: String): String {
        val uri = URI(gateway)
        val scheme = if (uri.scheme.equals("https", ignoreCase = true)) "wss" else "ws"
        return URI(scheme, uri.userInfo, uri.host, uri.port, REALTIME_PATH, null, null).toString()
    }

    private fun randomValue(bytes: Int): String {
        val value = ByteArray(bytes)
        random.nextBytes(value)
        return LumeTransportCrypto.encode(value)
    }

    companion object {
        private const val NOTIFICATION_ID = 6200
        private const val SERVICE_CHANNEL_ID = "lume-background-monitoring"
        private const val REALTIME_PATH = "/api/v1/ws"
        private const val REALTIME_SUBPROTOCOL = "lume.hub.v1"
        private const val DISCOVERY_INTERVAL_MS = 15_000L

        fun start(context: Context): Result<Unit> = runCatching {
            ContextCompat.startForegroundService(
                context.applicationContext,
                Intent(context.applicationContext, LumeRealtimeService::class.java),
            )
        }

        fun stop(context: Context) {
            context.applicationContext.stopService(
                Intent(context.applicationContext, LumeRealtimeService::class.java),
            )
        }
    }
}
