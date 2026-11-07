package name.apk.lanmsg.core

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** 对密钥字节做可逆包装；生产用 AndroidKeyStore，测试可替换。 */
interface KeyWrapper {
    fun encrypt(plain: ByteArray): ByteArray
    fun decrypt(blob: ByteArray): ByteArray
}

/** AES/GCM/NoPadding，主密钥存 AndroidKeyStore。格式：[12 字节 IV][密文+tag]。 */
class AndroidKeystoreWrapper : KeyWrapper {
    // 缓存已取到的主密钥；加锁防止两个线程同时首次生成，后者覆盖前者导致旧密钥文件永久解不开
    @Volatile
    private var cachedKey: SecretKey? = null

    @Synchronized
    private fun masterKey(): SecretKey {
        cachedKey?.let { return it }
        val key = loadOrCreateKey()
        cachedKey = key
        return key
    }

    private fun loadOrCreateKey(): SecretKey {
        val ks = KeyStore.getInstance(PROVIDER).apply { load(null) }
        (ks.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, PROVIDER)
        gen.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return gen.generateKey()
    }

    override fun encrypt(plain: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORM)
        cipher.init(Cipher.ENCRYPT_MODE, masterKey())
        return cipher.iv + cipher.doFinal(plain)
    }

    override fun decrypt(blob: ByteArray): ByteArray {
        require(blob.size > IV_LEN) { "Ciphertext too short" }
        val cipher = Cipher.getInstance(TRANSFORM)
        cipher.init(Cipher.DECRYPT_MODE, masterKey(), GCMParameterSpec(128, blob, 0, IV_LEN))
        return cipher.doFinal(blob, IV_LEN, blob.size - IV_LEN)
    }

    private companion object {
        const val PROVIDER = "AndroidKeyStore"
        const val ALIAS = "lanmsg_master"
        const val TRANSFORM = "AES/GCM/NoPadding"
        const val IV_LEN = 12
    }
}
