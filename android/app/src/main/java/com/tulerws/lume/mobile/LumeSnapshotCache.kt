package com.tulerws.lume.mobile

import android.content.Context
import androidx.room.Dao
import androidx.room.Database
import androidx.room.Entity
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.PrimaryKey
import androidx.room.Query
import androidx.room.Room
import androidx.room.RoomDatabase

@Entity(tableName = "lume_snapshot_cache")
internal data class LumeCachedSnapshot(
    @PrimaryKey
    val deviceId: String,
    val encryptedSnapshot: ByteArray,
    val updatedAt: Long,
)

@Dao
internal interface LumeSnapshotDao {
    @Query("SELECT * FROM lume_snapshot_cache WHERE deviceId = :deviceId LIMIT 1")
    suspend fun find(deviceId: String): LumeCachedSnapshot?

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun save(snapshot: LumeCachedSnapshot)

    @Query("DELETE FROM lume_snapshot_cache WHERE deviceId != :deviceId")
    suspend fun removeOtherDevices(deviceId: String)

    @Query("DELETE FROM lume_snapshot_cache")
    suspend fun clear()
}

@Database(
    entities = [LumeCachedSnapshot::class],
    version = 1,
    exportSchema = false,
)
internal abstract class LumeCacheDatabase : RoomDatabase() {
    abstract fun snapshots(): LumeSnapshotDao
}

internal data class LumeRestoredSnapshot(
    val json: String,
    val updatedAt: Long,
)

internal class LumeSnapshotCache(context: Context) {
    private val database = Room.databaseBuilder(
        context.applicationContext,
        LumeCacheDatabase::class.java,
        "lume_native_cache.db",
    ).build()
    private val cipher = LumeKeystoreCipher("lume.native.snapshot-cache.v1")

    suspend fun read(deviceId: String): LumeRestoredSnapshot? {
        val cached = database.snapshots().find(deviceId) ?: return null
        val cleartext = cipher.decrypt(cached.encryptedSnapshot) ?: return null
        return LumeRestoredSnapshot(
            json = cleartext.toString(Charsets.UTF_8),
            updatedAt = cached.updatedAt,
        )
    }

    suspend fun save(deviceId: String, snapshot: String) {
        val cleartext = snapshot.toByteArray(Charsets.UTF_8)
        if (cleartext.size > MAX_SNAPSHOT_BYTES) return
        database.snapshots().save(
            LumeCachedSnapshot(
                deviceId = deviceId,
                encryptedSnapshot = cipher.encrypt(cleartext),
                updatedAt = System.currentTimeMillis(),
            ),
        )
        database.snapshots().removeOtherDevices(deviceId)
    }

    suspend fun clear() {
        database.snapshots().clear()
        cipher.clear()
    }

    fun close() {
        database.close()
    }

    private companion object {
        const val MAX_SNAPSHOT_BYTES = 5 * 1024 * 1024
    }
}
