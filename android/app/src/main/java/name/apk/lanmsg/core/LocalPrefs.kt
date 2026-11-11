package name.apk.lanmsg.core

import android.content.Context
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.longPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.core.stringSetPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map
import uniffi.lanmsg_api.BackgroundMode

private val Context.dataStore by preferencesDataStore(name = "lanmsg_local")

/** 免打扰集合的窄接口，便于 ViewModel 测试注入内存实现。 */
interface MutedStore {
    val mutedConversations: Flow<Set<String>>

    suspend fun setMuted(conv: String, muted: Boolean)
}

/** 设置页用到的本机偏好窄接口，便于 ViewModel 测试注入内存实现。 */
interface SettingsPrefs {
    val notificationsEnabled: Flow<Boolean>
    val saveMediaToGallery: Flow<Boolean>
    val trustedWifi: Flow<Set<String>>

    suspend fun setNotificationsEnabled(v: Boolean)

    suspend fun setSaveMediaToGallery(v: Boolean)

    suspend fun setTrustedWifi(ssids: Set<String>)
}

/** Notifier 需要的本机偏好（免打扰、通知总开关、相册保存），便于测试注入内存实现。 */
interface NotifierPrefs : MutedStore, SettingsPrefs

/** 上次已知的后台档位，开机时服务启动前来不及读 core，先靠它决定是否立即拉起服务。 */
interface BackgroundModePrefs {
    val lastBackgroundMode: Flow<BackgroundMode?>

    suspend fun setLastBackgroundMode(mode: BackgroundMode)
}

/** 更新自动检查的节流时间戳，便于 ViewModel 测试注入内存实现。 */
interface UpdatePrefs {
    /** 上次自动检查更新的时间（epoch 毫秒）；null = 从未检查过。 */
    val lastAutoUpdateCheck: Flow<Long?>

    suspend fun setLastAutoUpdateCheck(millis: Long)
}

/** 只属于本机、不进 core 的偏好（免打扰、相册保存、可信 Wi-Fi、通知开关）。 */
class LocalPrefs(ctx: Context) : NotifierPrefs, BackgroundModePrefs, UpdatePrefs {
    private val store = ctx.applicationContext.dataStore

    override val mutedConversations: Flow<Set<String>> = store.data.map { it[MUTED] ?: emptySet() }

    override suspend fun setMuted(conv: String, muted: Boolean) {
        store.edit { p ->
            val cur = p[MUTED] ?: emptySet()
            p[MUTED] = if (muted) cur + conv else cur - conv
        }
    }

    override val saveMediaToGallery: Flow<Boolean> = store.data.map { it[SAVE_GALLERY] ?: true }

    override suspend fun setSaveMediaToGallery(v: Boolean) {
        store.edit { it[SAVE_GALLERY] = v }
    }

    override val trustedWifi: Flow<Set<String>> = store.data.map { it[TRUSTED_WIFI] ?: emptySet() }

    override suspend fun setTrustedWifi(ssids: Set<String>) {
        store.edit { it[TRUSTED_WIFI] = ssids }
    }

    override val notificationsEnabled: Flow<Boolean> = store.data.map { it[NOTIFY] ?: true }

    override suspend fun setNotificationsEnabled(v: Boolean) {
        store.edit { it[NOTIFY] = v }
    }

    /** 是否已经弹过 POST_NOTIFICATIONS 权限框：只弹一次，拒绝后不再反复弹。 */
    val notificationPermissionAsked: Flow<Boolean> = store.data.map { it[NOTIF_ASKED] ?: false }

    suspend fun setNotificationPermissionAsked() {
        store.edit { it[NOTIF_ASKED] = true }
    }

    /** 用户是否已点过“打开应用详情”：之后厂商省电指引折叠成一行。 */
    val vendorHintSeen: Flow<Boolean> = store.data.map { it[VENDOR_HINT_SEEN] ?: false }

    suspend fun setVendorHintSeen() {
        store.edit { it[VENDOR_HINT_SEEN] = true }
    }

    override val lastBackgroundMode: Flow<BackgroundMode?> = store.data.map { p ->
        p[LAST_MODE]?.let { name -> BackgroundMode.entries.firstOrNull { it.name == name } }
    }

    override suspend fun setLastBackgroundMode(mode: BackgroundMode) {
        store.edit { it[LAST_MODE] = mode.name }
    }

    override val lastAutoUpdateCheck: Flow<Long?> = store.data.map { it[LAST_UPDATE_CHECK] }

    override suspend fun setLastAutoUpdateCheck(millis: Long) {
        store.edit { it[LAST_UPDATE_CHECK] = millis }
    }

    private companion object {
        val LAST_MODE = stringPreferencesKey("last_background_mode")
        val MUTED = stringSetPreferencesKey("muted_conversations")
        val SAVE_GALLERY = booleanPreferencesKey("save_media_to_gallery")
        val TRUSTED_WIFI = stringSetPreferencesKey("trusted_wifi")
        val NOTIFY = booleanPreferencesKey("notifications_enabled")
        val VENDOR_HINT_SEEN = booleanPreferencesKey("vendor_hint_seen")
        val NOTIF_ASKED = booleanPreferencesKey("notification_permission_asked")
        val LAST_UPDATE_CHECK = longPreferencesKey("last_auto_update_check")
    }
}
