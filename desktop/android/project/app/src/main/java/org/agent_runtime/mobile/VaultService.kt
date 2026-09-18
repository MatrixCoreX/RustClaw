package org.agent_runtime.mobile

import android.app.Service
import android.content.*
import android.os.*
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

class VaultService : Service() {
    private val started = AtomicBoolean()
    override fun onCreate() { super.onCreate(); NativeBridge.initialize(this) }
    private val endpoint = object : Binder() {
        override fun onTransact(code: Int, data: Parcel, reply: Parcel?, flags: Int): Boolean {
            if (code != FIRST_CALL_TRANSACTION) return super.onTransact(code,data,reply,flags)
            check(getCallingUid() == Process.myUid())
            data.enforceInterface("agent-runtime.wallet.v1")
            check(started.compareAndSet(false,true))
            val guardian = requireNotNull(data.readStrongBinder())
            val socket = requireNotNull(ParcelFileDescriptor.CREATOR.createFromParcel(data))
            guardian.linkToDeath({ Process.killProcess(Process.myPid()) },0)
            val fd = socket.detachFd()
            Thread({ try { NativeBridge.runWorker(fd) } finally { Process.killProcess(Process.myPid()) } },"asset-vault").start()
            reply?.writeNoException(); return true
        }
    }
    override fun onBind(intent: Intent): IBinder = endpoint
    override fun onUnbind(intent: Intent): Boolean { Process.killProcess(Process.myPid()); return false }
}

object VaultConnection {
    private var connection: ServiceConnection? = null
    private val guardian = Binder()
    @Synchronized fun start(context: Context): Int {
        check(connection == null)
        val ready = CountDownLatch(1); val sockets = ParcelFileDescriptor.createSocketPair()
        var ok = false
        val candidate = object : ServiceConnection {
            override fun onServiceConnected(name: ComponentName, service: IBinder) {
                val data = Parcel.obtain(); val reply = Parcel.obtain()
                try {
                    data.writeInterfaceToken("agent-runtime.wallet.v1"); data.writeStrongBinder(guardian)
                    sockets[1].writeToParcel(data,0)
                    check(service.transact(IBinder.FIRST_CALL_TRANSACTION,data,reply,0))
                    reply.readException(); ok = true
                } catch (_: Exception) { ok = false } finally { data.recycle(); reply.recycle(); sockets[1].close(); ready.countDown() }
            }
            override fun onServiceDisconnected(name: ComponentName) { sockets[0].close() }
            override fun onNullBinding(name: ComponentName) { ready.countDown() }
        }
        connection = candidate
        Handler(Looper.getMainLooper()).post {
            if (!context.bindService(Intent(context,VaultService::class.java),candidate,Context.BIND_AUTO_CREATE)) ready.countDown()
        }
        if (!ready.await(20,TimeUnit.SECONDS) || !ok) {
            sockets.forEach { it.close() }; stop(context); error("wallet_worker_unavailable")
        }
        return sockets[0].detachFd()
    }
    @Synchronized fun stop(context: Context) {
        val current = connection ?: return; connection = null
        Handler(Looper.getMainLooper()).post { context.unbindService(current) }
    }
}
