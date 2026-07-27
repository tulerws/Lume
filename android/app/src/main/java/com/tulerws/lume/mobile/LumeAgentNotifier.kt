package com.tulerws.lume.mobile

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import androidx.core.content.edit
import org.json.JSONObject

internal class LumeAgentNotifier(context: Context) {
    private val applicationContext = context.applicationContext
    private val state = applicationContext.getSharedPreferences(STATE_STORE, Context.MODE_PRIVATE)

    fun observe(snapshot: JSONObject) {
        synchronized(notificationLock) {
            observeLocked(snapshot)
        }
    }

    fun clear() {
        synchronized(notificationLock) {
            state.edit {
                remove(STATUSES)
                .remove(PERMISSIONS)
            }
        }
    }

    private fun observeLocked(snapshot: JSONObject) {
        val sessions = snapshot.optJSONArray("sessions") ?: return
        val previous = decodeStatuses(state.getString(STATUSES, null))
        val previousPermissions = decodeStatuses(state.getString(PERMISSIONS, null))
        val current = linkedMapOf<String, String>()
        val currentPermissions = linkedMapOf<String, String>()
        for (index in 0 until sessions.length()) {
            val session = sessions.optJSONObject(index) ?: continue
            val id = session.optString("id").takeIf(String::isNotBlank) ?: continue
            val status = session.optString("status")
            current[id] = status
            val permissionId = session
                .optJSONObject("pendingPermission")
                ?.optString("id")
                ?.takeIf(String::isNotBlank)
            if (permissionId != null) {
                currentPermissions[id] = permissionId
            } else if (previousPermissions[id] != null) {
                currentPermissions[id] = previousPermissions.getValue(id)
            }
            val isNewPermission = status == "permission_required" &&
                permissionId != null &&
                previousPermissions[id] != permissionId
            val isStatusTransition = status != "permission_required" &&
                previous[id] != null &&
                previous[id] != status
            if (previous.isNotEmpty() && (isNewPermission || isStatusTransition)) {
                notifyTransition(session, status)
            }
        }
        state.edit {
            putString(STATUSES, JSONObject(current as Map<*, *>).toString())
                .putString(PERMISSIONS, JSONObject(currentPermissions as Map<*, *>).toString())
        }
    }

    private fun notifyTransition(session: JSONObject, status: String) {
        if (!LumeNativePreferences.notificationsEnabled(applicationContext)) return
        val body = when (status) {
            "permission_required" -> "Needs a permission decision."
            "completed" -> "Finished a task."
            "failed" -> "Reported an error."
            else -> return
        }
        if (
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            ContextCompat.checkSelfPermission(
                applicationContext,
                Manifest.permission.POST_NOTIFICATIONS,
            ) != PackageManager.PERMISSION_GRANTED
        ) {
            return
        }
        ensureChannel()
        val sessionId = session.optString("id")
        val agent = session.optString("agentLabel", "AI agent")
        val project = session.optString("projectName")
        val detail = if (project.isBlank()) body else "$body · $project"
        val intent = Intent(applicationContext, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        val pendingIntent = PendingIntent.getActivity(
            applicationContext,
            notificationId("$sessionId:$status"),
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val notification = NotificationCompat.Builder(applicationContext, CHANNEL_ID)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle(agent)
            .setContentText(detail)
            .setStyle(NotificationCompat.BigTextStyle().bigText(detail))
            .setContentIntent(pendingIntent)
            .setAutoCancel(true)
            .setOnlyAlertOnce(true)
            .setPriority(
                if (status == "permission_required") {
                    NotificationCompat.PRIORITY_HIGH
                } else {
                    NotificationCompat.PRIORITY_DEFAULT
                },
            )
            .build()
        NotificationManagerCompat.from(applicationContext)
            .notify(notificationId("$sessionId:$status"), notification)
    }

    private fun ensureChannel() {
        val manager = applicationContext.getSystemService(NotificationManager::class.java)
        val channel = NotificationChannel(
            CHANNEL_ID,
            "Agent activity",
            NotificationManager.IMPORTANCE_HIGH,
        ).apply {
            description = "Task completion, errors and permission requests"
        }
        manager.createNotificationChannel(channel)
    }

    private fun decodeStatuses(value: String?): Map<String, String> {
        if (value.isNullOrBlank()) return emptyMap()
        val root = runCatching { JSONObject(value) }.getOrNull() ?: return emptyMap()
        return root.keys().asSequence().associateWith(root::optString)
    }

    private fun notificationId(value: String): Int =
        value.fold(17) { result, character ->
            ((result * 31) + character.code) and Int.MAX_VALUE
        }.coerceAtLeast(1)

    private companion object {
        const val CHANNEL_ID = "lume-agent-events"
        const val STATE_STORE = "lume-native-agent-notification-state"
        const val STATUSES = "session-statuses"
        const val PERMISSIONS = "session-permissions"
        val notificationLock = Any()
    }
}
