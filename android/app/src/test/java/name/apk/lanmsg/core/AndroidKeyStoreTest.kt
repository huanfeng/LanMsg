package name.apk.lanmsg.core

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import uniffi.lanmsg_ffi.FfiException
import java.io.File

class AndroidKeyStoreTest {
    private val xorWrapper = object : KeyWrapper { // 测试用可逆包装，验证文件流程
        override fun encrypt(plain: ByteArray) = plain.map { (it.toInt() xor 0x5a).toByte() }.toByteArray()
        override fun decrypt(blob: ByteArray) = encrypt(blob)
    }

    @get:Rule val tmp = TemporaryFolder()

    @Test fun save_load_delete_roundtrip() {
        val ks = AndroidKeyStore(tmp.root, xorWrapper)
        assertNull(ks.load("device.key"))
        ks.save("device.key", byteArrayOf(1, 2, 3))
        assertArrayEquals(byteArrayOf(1, 2, 3), ks.load("device.key"))
        assertFalse(File(tmp.root, "device.key.bin").readBytes().contentEquals(byteArrayOf(1, 2, 3)))
        ks.delete("device.key")
        assertNull(ks.load("device.key"))
    }

    @Test fun corrupted_file_raises_api_error() {
        val failing = object : KeyWrapper {
            override fun encrypt(plain: ByteArray) = plain
            override fun decrypt(blob: ByteArray): ByteArray = throw javax.crypto.AEADBadTagException()
        }
        val ks = AndroidKeyStore(tmp.root, failing)
        ks.save("user.key", byteArrayOf(9))
        assertThrows(FfiException.Api::class.java) { ks.load("user.key") }
    }
}
