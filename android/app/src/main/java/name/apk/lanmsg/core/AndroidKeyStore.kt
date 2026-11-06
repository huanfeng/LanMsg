package name.apk.lanmsg.core

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
            throw FfiException.Api(ErrorCode.INTERNAL, "密钥文件损坏: $name")
        }
    }

    override fun save(name: String, bytes: ByteArray) {
        try {
            dir.mkdirs()
            val tmp = File(dir, "$name.bin.tmp")
            tmp.writeBytes(wrapper.encrypt(bytes))
            if (!tmp.renameTo(file(name))) {
                tmp.delete()
                throw java.io.IOException("重命名失败")
            }
        } catch (e: Exception) {
            throw FfiException.Api(ErrorCode.IO, "保存密钥失败: $name")
        }
    }

    override fun delete(name: String) {
        file(name).delete()
    }
}
