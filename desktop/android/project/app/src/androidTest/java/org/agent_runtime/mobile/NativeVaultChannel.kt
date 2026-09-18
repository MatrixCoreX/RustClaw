package org.agent_runtime.mobile

import android.app.ActivityManager
import android.content.Context
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import org.json.JSONObject
import org.junit.Assert.*
import java.io.*

/** A private test connection; no production command or export surface is added. */
class NativeVaultChannel(private val context:Context):Closeable {
    private val owned=ParcelFileDescriptor.adoptFd(NativeBridge.startWorker())
    private val output=DataOutputStream(ParcelFileDescriptor.AutoCloseOutputStream(ParcelFileDescriptor.dup(owned.fileDescriptor)))
    private val input=DataInputStream(ParcelFileDescriptor.AutoCloseInputStream(owned))
    private var sequence=0L
    init {
        android.system.Os.setsockoptTimeval(owned.fileDescriptor,android.system.OsConstants.SOL_SOCKET,
            android.system.OsConstants.SO_RCVTIMEO,android.system.StructTimeval.fromMillis(90000))
    }
    fun call(request:JSONObject):JSONObject {
        val bytes=JSONObject().put("version",1).put("id",++sequence).put("request",request).toString().toByteArray()
        output.writeInt(bytes.size);output.write(bytes);output.flush()
        val length=input.readInt();assertTrue(length in 1..131072)
        val buffer=ByteArray(length);input.readFully(buffer)
        val response=JSONObject(String(buffer));assertEquals(sequence,response.getLong("id"))
        return response.getJSONObject("result")
    }
    override fun close() {
        input.close();output.close();NativeBridge.stopWorker()
        val end=SystemClock.elapsedRealtime()+10000
        val manager=context.getSystemService(ActivityManager::class.java)
        while(manager.runningAppProcesses.orEmpty().any { it.processName==context.packageName+":asset_vault" }
            && SystemClock.elapsedRealtime()<end) Thread.sleep(50)
        check(manager.runningAppProcesses.orEmpty().none { it.processName==context.packageName+":asset_vault" })
    }
}
