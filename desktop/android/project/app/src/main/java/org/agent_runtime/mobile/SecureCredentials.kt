package org.agent_runtime.mobile

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import java.io.File
import java.io.RandomAccessFile
import java.nio.ByteBuffer
import java.security.KeyStore
import java.security.MessageDigest
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Keystore keys cannot be exported. Ciphertexts are in noBackupFilesDir only. */
object SecureCredentials {
    private const val ALIAS = "agent-runtime.mobile.credentials.v1"
    @Synchronized private fun key(context: Context, create: Boolean): SecretKey {
        // Multiple app processes share the alias. Serialize first creation so
        // concurrent wallet/login saves can never replace one another's key.
        RandomAccessFile(File(context.noBackupFilesDir,"credential-key.lock"),"rw").use { lockFile ->
          lockFile.channel.lock().use {
            return keyLocked(create)
          }
        }
    }
    private fun keyLocked(create: Boolean): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        if (store.containsAlias(ALIAS)) {
            return requireNotNull(store.getKey(ALIAS,null) as? SecretKey) { "credential_key_invalid" }
        }
        check(create) { "credential_key_missing" }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
            init(KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256).setRandomizedEncryptionRequired(true).build())
        }.generateKey()
    }
    private fun aad(service: String, id: String): ByteArray {
        require(service.length in 1..100 && id.length in 1..100)
        require('\u0000' !in service && '\u0000' !in id)
        return "$service\u0000$id".toByteArray(Charsets.UTF_8)
    }
    private fun path(context: Context, service: String, id: String): File {
        val name = MessageDigest.getInstance("SHA-256").digest(aad(service, id)).joinToString("") { "%02x".format(it) }
        val dir = File(context.noBackupFilesDir, "credentials-v1").apply { check(isDirectory || mkdir()) }
        return File(dir, name)
    }
    @Synchronized fun put(context: Context, service: String, id: String, value: String) {
        val bytes = value.toByteArray(Charsets.UTF_8)
        require(bytes.size <= 131072)
        try {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.ENCRYPT_MODE, key(context,true)); cipher.updateAAD(aad(service,id))
            val encrypted = cipher.doFinal(bytes)
            val record = ByteBuffer.allocate(1 + 12 + encrypted.size).put(1).put(cipher.iv).put(encrypted).array()
            val target = AtomicFile(path(context,service,id)); val out = target.startWrite()
            try { out.write(record); target.finishWrite(out) } catch (e: Exception) { target.failWrite(out); throw e }
        } finally { bytes.fill(0) }
    }
    @Synchronized fun get(context: Context, service: String, id: String): String? {
        val file = path(context,service,id)
        if (!file.exists()) return null
        require(file.length() in 30..131200)
        val record = AtomicFile(file).readFully(); require(record[0] == 1.toByte())
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE,key(context,false),GCMParameterSpec(128,record.copyOfRange(1,13)))
        cipher.updateAAD(aad(service,id))
        val bytes = cipher.doFinal(record,13,record.size-13)
        return try { String(bytes,Charsets.UTF_8) } finally { bytes.fill(0) }
    }
    @Synchronized fun delete(context: Context, service: String, id: String) { AtomicFile(path(context,service,id)).delete() }
}
