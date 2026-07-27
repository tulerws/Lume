package com.tulerws.lume.mobile

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull
import java.net.Inet4Address
import java.net.InetAddress
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.coroutines.resume

internal class LumeDiscovery(context: Context) {
    private val applicationContext = context.applicationContext
    private val nsdManager = applicationContext.getSystemService(NsdManager::class.java)
    private val wifiManager = applicationContext.getSystemService(WifiManager::class.java)

    suspend fun findGateway(desktopId: String, timeoutMillis: Long = 4_000): String? =
        withTimeoutOrNull(timeoutMillis) {
            discover(desktopId)
        }

    private suspend fun discover(desktopId: String): String? =
        suspendCancellableCoroutine { continuation ->
            val finished = AtomicBoolean(false)
            val resolving = AtomicBoolean(false)
            val multicastLock = wifiManager?.createMulticastLock("lume-mobile-discovery")?.apply {
                setReferenceCounted(false)
                runCatching { acquire() }
            }
            lateinit var listener: NsdManager.DiscoveryListener

            fun finish(value: String?) {
                if (!finished.compareAndSet(false, true)) return
                runCatching { nsdManager.stopServiceDiscovery(listener) }
                if (multicastLock?.isHeld == true) runCatching { multicastLock.release() }
                if (continuation.isActive) continuation.resume(value)
            }

            fun addresses(service: NsdServiceInfo): List<InetAddress> =
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                    service.hostAddresses
                } else {
                    @Suppress("DEPRECATION")
                    listOfNotNull(service.host)
                }

            val resolver = object : NsdManager.ResolveListener {
                override fun onResolveFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
                    resolving.set(false)
                }

                override fun onServiceResolved(serviceInfo: NsdServiceInfo) {
                    val advertisedId = serviceInfo.attributes["id"]
                        ?.toString(Charsets.UTF_8)
                    if (advertisedId != desktopId) {
                        resolving.set(false)
                        return
                    }
                    val address = addresses(serviceInfo)
                        .filterNot { it.isLoopbackAddress || it.isAnyLocalAddress }
                        .let { candidates ->
                            candidates.firstOrNull { it is Inet4Address }
                                ?: candidates.firstOrNull()
                        }
                    val host = address?.hostAddress
                    if (host.isNullOrBlank() || serviceInfo.port <= 0) {
                        resolving.set(false)
                        return
                    }
                    val authority = if (host.contains(':')) "[$host]" else host
                    finish("http://$authority:${serviceInfo.port}")
                }
            }

            listener = object : NsdManager.DiscoveryListener {
                override fun onDiscoveryStarted(serviceType: String) = Unit

                override fun onServiceFound(serviceInfo: NsdServiceInfo) {
                    if (
                        !serviceInfo.serviceType.startsWith("_lume._tcp") ||
                        !resolving.compareAndSet(false, true)
                    ) {
                        return
                    }
                    @Suppress("DEPRECATION")
                    nsdManager.resolveService(serviceInfo, resolver)
                }

                override fun onServiceLost(serviceInfo: NsdServiceInfo) = Unit

                override fun onDiscoveryStopped(serviceType: String) {
                    finish(null)
                }

                override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                    finish(null)
                }

                override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {
                    finish(null)
                }
            }

            continuation.invokeOnCancellation { finish(null) }
            runCatching {
                nsdManager.discoverServices(
                    SERVICE_TYPE,
                    NsdManager.PROTOCOL_DNS_SD,
                    listener,
                )
            }.onFailure { finish(null) }
        }

    private companion object {
        const val SERVICE_TYPE = "_lume._tcp."
    }
}
