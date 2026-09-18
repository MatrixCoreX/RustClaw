package org.agent_runtime.mobile

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.util.AtomicFile
import java.io.File
import java.lang.ref.WeakReference
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.CompletableFuture
import java.util.concurrent.TimeUnit

/** Storage Access Framework: no broad storage permission and no raw private-key export. */
object Documents {
    private var activity = WeakReference<Activity>(null)
    @Volatile private var pending: CompletableFuture<Uri?>? = null
    fun choosing(): Boolean = pending != null
    private const val REQUEST = 7412
    fun attach(value: Activity) { activity = WeakReference(value) }
    fun result(request: Int, code: Int, intent: Intent?): Boolean {
        if (request != REQUEST) return false
        val future = pending; pending = null
        future?.complete(if (code == Activity.RESULT_OK) intent?.data else null)
        return true
    }
    private fun choose(intent: Intent): Uri? {
        check(Looper.myLooper() != Looper.getMainLooper())
        val future = CompletableFuture<Uri?>()
        Handler(Looper.getMainLooper()).post {
            if (pending != null) future.completeExceptionally(IllegalStateException("document_busy"))
            else {
                pending = future
                try { requireNotNull(activity.get()).startActivityForResult(intent,REQUEST) }
                catch (e: Exception) { pending=null; future.completeExceptionally(e) }
            }
        }
        return try { future.get(5,TimeUnit.MINUTES)?.also { require(it.scheme == "content") } }
        finally { Handler(Looper.getMainLooper()).post { if(pending === future) pending = null } }
    }
    private fun directory() = File(NativeBridge.context.cacheDir,"document-transfer").apply { check(isDirectory || mkdir()) }
    fun create(name: String): String? {
        val uri = choose(Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE); type="application/octet-stream"
            putExtra(Intent.EXTRA_TITLE,name.take(180))
        }) ?: return null
        val path = File(directory(), UUID.randomUUID().toString())
        File(path.path+".destination").writeText(uri.toString())
        return path.path
    }
    fun pick(): String? {
        val uri = choose(Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE); type="*/*"
        }) ?: return null
        val path = File(directory(),UUID.randomUUID().toString())
        NativeBridge.context.contentResolver.openInputStream(uri).use { input ->
            requireNotNull(input)
            path.outputStream().use { out ->
                val buffer=ByteArray(8192); var total=0
                while (true) { val n=input.read(buffer); if(n<0) break; total+=n; require(total<=1_000_000); out.write(buffer,0,n) }
            }
        }
        return path.path
    }
    fun finish(value: String) {
        val path=File(value).canonicalFile
        require(path.parentFile == directory().canonicalFile && path.isFile)
        val mapping=File(path.path+".destination"); val uri=Uri.parse(mapping.readText())
        require(uri.scheme=="content")
        val resolver=NativeBridge.context.contentResolver
        val digest=MessageDigest.getInstance("SHA-256")
        resolver.openOutputStream(uri,"wt").use { out ->
            requireNotNull(out)
            path.inputStream().use { input ->
                val b=ByteArray(65536)
                while(true) { val n=input.read(b); if(n<0) break; out.write(b,0,n); digest.update(b,0,n) }
            }
            out.flush()
        }
        val verify=MessageDigest.getInstance("SHA-256")
        resolver.openInputStream(uri).use { input ->
            requireNotNull(input); val b=ByteArray(65536);var total=0L
            while(true) { val n=input.read(b); if(n<0) break;total+=n;check(total<=path.length());verify.update(b,0,n) }
            check(total==path.length())
        }
        check(MessageDigest.isEqual(digest.digest(),verify.digest())) { "document_verify_failed" }
        check(mapping.delete()); check(path.delete())
    }
    fun external(url: String) {
        val uri=Uri.parse(url); require(uri.scheme in listOf("http","https","mailto"))
        Handler(Looper.getMainLooper()).post { activity.get()?.startActivity(Intent(Intent.ACTION_VIEW,uri)) }
    }
    fun show(type: Class<out Activity>) {
        Handler(Looper.getMainLooper()).post { activity.get()?.let { it.startActivity(Intent(it,type).addFlags(Intent.FLAG_ACTIVITY_REORDER_TO_FRONT)) } }
    }
}
