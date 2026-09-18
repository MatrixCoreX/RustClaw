package org.agent_runtime.mobile

import android.app.KeyguardManager
import android.content.Context
import android.os.PowerManager
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.ProcessLifecycleOwner

/** No Javascript interface is exposed. Only the native Rust host calls this API. */
object NativeBridge {
    lateinit var context: Context
        private set
    init { System.loadLibrary("agent_desktop") }
    @JvmStatic external fun nativeInit()
    @JvmStatic external fun runWorker(fd: Int)
    fun initialize(context: Context) {
        this.context = context.applicationContext
        nativeInit()
    }
    @JvmStatic fun startWorker(): Int = VaultConnection.start(context)
    @JvmStatic fun stopWorker() = VaultConnection.stop(context)
    @JvmStatic fun isLocked(): Boolean =
        context.getSystemService(KeyguardManager::class.java).isDeviceLocked ||
        !context.getSystemService(PowerManager::class.java).isInteractive ||
        (!Documents.choosing() && !ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED))
    @JvmStatic fun credentialGet(service: String, id: String): String? = SecureCredentials.get(context, service, id)
    @JvmStatic fun credentialPut(service: String, id: String, value: String): String? {
        SecureCredentials.put(context, service, id, value); return null
    }
    @JvmStatic fun credentialDelete(service: String, id: String): String? {
        SecureCredentials.delete(context, service, id); return null
    }
    @JvmStatic fun localNetworks(): String = NetworkDiscovery.networks(context)
    @JvmStatic fun discoverMdns(): String = NetworkDiscovery.mdns(context)
    @JvmStatic fun pickBackup(): String? = Documents.pick()
    @JvmStatic fun saveDocument(name: String): String? = Documents.create(name)
    @JvmStatic fun finishDocument(path: String): String? { Documents.finish(path); return null }
    @JvmStatic fun openExternal(url: String): String? { Documents.external(url); return null }
    @JvmStatic fun showWallet(): String? { Documents.show(WalletActivity::class.java); return null }
}
