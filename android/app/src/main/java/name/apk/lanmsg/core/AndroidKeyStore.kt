package name.apk.lanmsg.core

import android.util.Log
import uniffi.lanmsg_api.ErrorCode
import uniffi.lanmsg_ffi.FfiException
import uniffi.lanmsg_ffi.FfiKeyStore
import java.io.File

/** 把 core 的密钥字节以 wrapper 加密后存成 `<dir>/<name>.bin`。 */
class AndroidKeyStore(private val dir: File, private val wrapper: KeyWrapper) : FfiKeyStore {
    private fun file(name: String) = File(dir, "$name.bin")

    override fun load(name: String): ByteArray? {
        val f = file(name)
        if (!f.exists()) return null
        return try {
            wrapper.decrypt(f.readBytes())
        } catch (e: Exception) {
            Log.w(TAG, "Failed to read key: $name", e)
            throw FfiException.Api(ErrorCode.INTERNAL, "Key file corrupted: $name")
        }
    }

    override fun save(name: String, bytes: ByteArray) {
        val tmp = File(dir, "$name.bin.tmp")
        try {
            dir.mkdirs()
            tmp.writeBytes(wrapper.encrypt(bytes))
            if (!tmp.renameTo(file(name))) throw java.io.IOException("Rename failed")
        } catch (e: Exception) {
            Log.w(TAG, "Failed to save key: $name", e)
            throw FfiException.Api(ErrorCode.IO, "Failed to save key: $name")
        } finally {
            tmp.delete() // 成功时已被重命名，delete 为空操作；失败时清掉残留
        }
    }

    override fun delete(name: String) {
        file(name).delete()
    }

    private companion object {
        const val TAG = "AndroidKeyStore"
    }
}
