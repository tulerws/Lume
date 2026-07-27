package com.tulerws.lume.mobile

import android.content.Context
import androidx.core.content.edit

internal object LumeNativePreferences {
    private const val STORE = "lume-native-monitoring"
    private const val NOTIFICATIONS_ENABLED = "notifications-enabled"
    private const val BACKGROUND_ENABLED = "background-enabled"

    fun notificationsEnabled(context: Context): Boolean =
        preferences(context).getBoolean(NOTIFICATIONS_ENABLED, false)

    fun backgroundEnabled(context: Context): Boolean =
        preferences(context).getBoolean(BACKGROUND_ENABLED, false)

    fun update(
        context: Context,
        notificationsEnabled: Boolean,
        backgroundEnabled: Boolean,
    ) {
        preferences(context).edit {
            putBoolean(NOTIFICATIONS_ENABLED, notificationsEnabled)
            putBoolean(BACKGROUND_ENABLED, notificationsEnabled && backgroundEnabled)
        }
    }

    private fun preferences(context: Context) =
        context.applicationContext.getSharedPreferences(STORE, Context.MODE_PRIVATE)
}
