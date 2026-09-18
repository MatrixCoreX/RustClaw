package org.agent_runtime.mobile

import android.app.Activity
import android.content.Intent
import android.content.IntentFilter
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.SystemClock
import android.util.Base64
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.webkit.WebView
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.core.content.FileProvider
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry
import androidx.test.runner.lifecycle.Stage
import org.json.JSONObject
import org.junit.Test
import org.junit.runner.RunWith
import java.io.ByteArrayOutputStream
import java.io.File
import java.net.InetAddress
import java.net.ServerSocket
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Test APK only. Bounded loopback harness drives the separately signed release APK. */
@RunWith(AndroidJUnit4::class)
class UiHarnessTest {
    private val runner=InstrumentationRegistry.getInstrumentation()
    private val activities=mutableMapOf<String,Activity>()
    private var selected="main"
    private var documentMonitor:android.app.Instrumentation.ActivityMonitor?=null
    private fun find(view: View): WebView? {
        if(view is WebView) return view
        if(view is ViewGroup) for(i in 0 until view.childCount) find(view.getChildAt(i))?.let { return it }
        return null
    }
    private fun refresh() = runner.runOnMainSync {
        for(stage in Stage.values()) for(activity in ActivityLifecycleMonitorRegistry.getInstance().getActivitiesInStage(stage)) {
            if(activity is MainActivity && !activity.isDestroyed) activities[if(activity is WalletActivity) "wallet" else "main"]=activity
        }
    }
    private fun view():WebView { refresh(); var result:WebView?=null
        runner.runOnMainSync { result=find(requireNotNull(activities[selected]).window.decorView) };return requireNotNull(result)
    }
    private fun evaluate(script:String):String {
        val view=view();val done=CountDownLatch(1);var value="null"
        runner.runOnMainSync { view.evaluateJavascript(script) { value=it;done.countDown() } }
        check(done.await(20,TimeUnit.SECONDS)) { "evaluation_timeout" };return value
    }
    @Test fun driveReleaseApplication() {
        val context=runner.targetContext
        runner.startActivitySync(Intent(context,MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        val end=SystemClock.elapsedRealtime()+30*60*1000
        ServerSocket(8765,1,InetAddress.getByName("127.0.0.1")).use { server ->
            server.soTimeout=20000
            var quit=false
            while(!quit && SystemClock.elapsedRealtime()<end) {
                val client=try { server.accept() } catch(_:java.net.SocketTimeoutException) { continue }
                client.use {
                    it.soTimeout=20000
                    val line=it.getInputStream().bufferedReader().readLine();require(line.length<1_000_000)
                    val request=JSONObject(line)
                    val response=try {
                        val value:Any?=when(request.getString("command")) {
                            "eval" -> evaluate(request.getString("script"))
                            "windows" -> { refresh(); activities.keys.toTypedArray().joinToString(",") }
                            "imeVisible" -> {
                                refresh();var visible=false
                                runner.runOnMainSync {
                                    val root=requireNotNull(activities[selected]).window.decorView
                                    visible=requireNotNull(androidx.core.view.ViewCompat.getRootWindowInsets(root))
                                        .isVisible(androidx.core.view.WindowInsetsCompat.Type.ime())
                                };visible
                            }
                            "clipboard" -> {
                                var text=""
                                runner.runOnMainSync { text=context.getSystemService(android.content.ClipboardManager::class.java).primaryClip?.getItemAt(0)?.text?.toString() ?: "" };text
                            }
                            "savedDocument" -> {
                                val file=File(context.cacheDir,"fixture-export.json")
                                if(file.exists() && file.length()<1_000_000) file.readText() else ""
                            }
                            "switch" -> {
                                selected=request.getString("window");refresh();val activity=requireNotNull(activities[selected])
                                runner.runOnMainSync { activity.startActivity(Intent(activity,activity.javaClass).addFlags(Intent.FLAG_ACTIVITY_REORDER_TO_FRONT)) };true
                            }
                            "tap" -> {
                                val view=view();val position=IntArray(2);runner.runOnMainSync { view.getLocationOnScreen(position) }
                                val x=position[0]+request.getDouble("x").toFloat();val y=position[1]+request.getDouble("y").toFloat()
                                val now=SystemClock.uptimeMillis()
                                for(action in listOf(MotionEvent.ACTION_DOWN,MotionEvent.ACTION_UP)) {
                                    val event=MotionEvent.obtain(now,SystemClock.uptimeMillis(),action,x,y,0)
                                    runner.sendPointerSync(event);event.recycle()
                                };true
                            }
                            "screenshot" -> {
                                val view=view();val output=ByteArrayOutputStream()
                                runner.runOnMainSync {
                                    val bitmap=Bitmap.createBitmap(view.width,view.height,Bitmap.Config.ARGB_8888)
                                    view.draw(Canvas(bitmap));bitmap.compress(Bitmap.CompressFormat.PNG,100,output);bitmap.recycle()
                                };Base64.encodeToString(output.toByteArray(),Base64.NO_WRAP)
                            }
                            "prepareDocument" -> {
                                documentMonitor?.let { runner.removeMonitor(it) }
                                val mode=request.getString("mode")
                                val path=File(context.cacheDir,if(mode=="open") "fixture-import.json" else "fixture-export.json")
                                if(mode=="open") runner.context.assets.open("wallet-linux-v2.json").use { source ->
                                    path.outputStream().use { out -> source.copyTo(out) }
                                } else check(mode=="save")
                                val uri=FileProvider.getUriForFile(context,context.packageName+".fileprovider",path)
                                val action=if(mode=="open") Intent.ACTION_OPEN_DOCUMENT else Intent.ACTION_CREATE_DOCUMENT
                                documentMonitor=runner.addMonitor(IntentFilter(action),android.app.Instrumentation.ActivityResult(Activity.RESULT_OK,Intent().setData(uri)),true)
                                true
                            }
                            "quit" -> { quit=true;true }
                            else -> error("unknown_test_command")
                        }
                        JSONObject().put("ok",true).put("value",value ?: JSONObject.NULL)
                    } catch(error:Exception) { JSONObject().put("ok",false).put("error",error.javaClass.simpleName+":"+error.message) }
                    it.getOutputStream().write((response.toString()+"\n").toByteArray())
                }
            }
            check(quit) { "harness_timeout" }
            documentMonitor?.let { runner.removeMonitor(it) }
        }
    }
}
