package org.agent_runtime.mobile

import android.os.Process
import android.content.Context
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Test
import org.junit.Assert.*
import org.junit.runner.RunWith
import java.io.File
import java.io.DataInputStream
import java.io.DataOutputStream
import android.os.ParcelFileDescriptor
import org.json.JSONObject
import java.util.UUID

@RunWith(AndroidJUnit4::class)
class NativeSecurityTest {
    private val context: Context get()=InstrumentationRegistry.getInstrumentation().targetContext
    @Test fun credentialsAreEncryptedBoundAndPersistent() {
        NativeBridge.initialize(context)
        val id=UUID.randomUUID().toString(); val value="Synthetic-test-value-"+UUID.randomUUID()
        assertNull(NativeBridge.credentialGet("native-test",id))
        NativeBridge.credentialPut("native-test",id,value)
        assertEquals(value,NativeBridge.credentialGet("native-test",id))
        assertNull(NativeBridge.credentialGet("other-test",id))
        assertThrows(IllegalArgumentException::class.java) { NativeBridge.credentialGet("native-test\u0000",id) }
        File(context.noBackupFilesDir,"credentials-v1").listFiles()!!.forEach { assertFalse(it.readBytes().toString(Charsets.UTF_8).contains(value)) }
        NativeBridge.credentialDelete("native-test",id)
        assertNull(NativeBridge.credentialGet("native-test",id))
    }
    @Test fun privateWorkerRejectsUnapprovedProtocol() {
        NativeBridge.initialize(context)
        val directory=File(context.cacheDir,"vault-test-"+UUID.randomUUID())
        val fd=NativeBridge.startWorker()
        val owned=ParcelFileDescriptor.adoptFd(fd)
        android.system.Os.setsockoptTimeval(owned.fileDescriptor,android.system.OsConstants.SOL_SOCKET,
            android.system.OsConstants.SO_RCVTIMEO,android.system.StructTimeval.fromMillis(15000))
        val input=DataInputStream(ParcelFileDescriptor.AutoCloseInputStream(owned))
        val duplicate=ParcelFileDescriptor.dup(owned.fileDescriptor)
        val output=DataOutputStream(ParcelFileDescriptor.AutoCloseOutputStream(duplicate))
        fun send(text: String): JSONObject {
            val bytes=text.toByteArray(); output.writeInt(bytes.size);output.write(bytes);output.flush()
            val len=input.readInt();assertTrue(len in 1..131072)
            val buffer=ByteArray(len);input.readFully(buffer);return JSONObject(String(buffer))
        }
        try {
            val open=send(JSONObject().put("version",1).put("id",1).put("request",JSONObject().put("operation","open").put("directory",directory.path)).toString())
            assertTrue(open.toString(),open.getJSONObject("result").has("Ok"))
            assertEquals(JSONObject.NULL,open.getJSONObject("result").get("Ok"))
            java.io.RandomAccessFile(File(directory,"vault.lock"),"rw").use { peer ->
                assertNull("another process acquired the open vault lock",peer.channel.tryLock())
            }
            val status=send("""{"version":1,"id":2,"request":{"operation":"status"}}""")
            assertFalse(status.getJSONObject("result").getJSONObject("Ok").getBoolean("unlocked"))
            // Unknown operations are rejected by the same closed Rust protocol.
            val bytes="""{"version":1,"id":3,"request":{"operation":"export_private_key"}}""".toByteArray()
            output.writeInt(bytes.size);output.write(bytes);output.flush()
            assertEquals(-1,input.read())
        } finally { input.close();output.close();NativeBridge.stopWorker() }
    }
}
