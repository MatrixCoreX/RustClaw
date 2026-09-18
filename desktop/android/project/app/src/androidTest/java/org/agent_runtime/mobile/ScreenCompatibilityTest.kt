package org.agent_runtime.mobile

import android.app.Activity
import android.content.Intent
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.webkit.WebView
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Test
import org.junit.Assert.*
import org.junit.runner.RunWith
import org.json.JSONObject
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

@RunWith(AndroidJUnit4::class)
class ScreenCompatibilityTest {
    private val instrumentation=InstrumentationRegistry.getInstrumentation()
    private fun webview(view: View): WebView? {
        if(view is WebView) return view
        if(view is ViewGroup) for(i in 0 until view.childCount) webview(view.getChildAt(i))?.let { return it }
        return null
    }
    private fun evaluate(view:WebView,script:String):String {
        val ready=CountDownLatch(1);var value=""
        instrumentation.runOnMainSync { view.evaluateJavascript(script) { value=it; ready.countDown() } }
        assertTrue("javascript_result_timeout",ready.await(15,TimeUnit.SECONDS));return value
    }
    @Test fun homeAndSecureAccountScreenFitViewport() {
        val context=instrumentation.targetContext
        val activity=instrumentation.startActivitySync(Intent(context,MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        var view:WebView?=null
        repeat(80) {
            instrumentation.runOnMainSync { view=webview(activity.window.decorView) }
            if(view!=null && evaluate(view!!,"document.querySelector('.desktop-home') !== null")=="true") return@repeat
            Thread.sleep(250)
        }
        val main=requireNotNull(view)
        assertEquals("home_did_not_load","true",evaluate(main,"document.querySelector('.desktop-home') !== null"))
        assertEquals("horizontal_overflow","true",evaluate(main,"document.documentElement.scrollWidth <= innerWidth + 1"))
        val result=evaluate(main,"JSON.stringify({width:innerWidth,height:innerHeight,dpr:devicePixelRatio,scrollWidth:document.documentElement.scrollWidth,theme:document.documentElement.dataset.theme})")
        File(context.getExternalFilesDir(null),"screen-home.json").writeText(result)
        assertTrue(activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE != 0)
        // Exercise the real native permission boundary from the main WebView.
        evaluate(main,"window.__testDenied=null;window.__TAURI_INTERNALS__.invoke('wallet_initialize',{password:'Synthetic-test-Phrase7!'}).then(()=>window.__testDenied=false,()=>window.__testDenied=true);null")
        repeat(40) { if(evaluate(main,"window.__testDenied")=="true") return@repeat;Thread.sleep(100) }
        assertEquals("main_wallet_permission_leak","true",evaluate(main,"window.__testDenied"))
        val monitor=instrumentation.addMonitor(WalletActivity::class.java.name,null,false)
        evaluate(main,"document.querySelector('.desktop-home-wallet button').click();null")
        val wallet=instrumentation.waitForMonitorWithTimeout(monitor,15000)
        instrumentation.removeMonitor(monitor)
        assertNotNull("wallet_activity_did_not_open",wallet)
        var secure:WebView?=null
        repeat(60) { instrumentation.runOnMainSync { secure=webview(wallet.window.decorView) };if(secure==null) Thread.sleep(100) }
        val secureView=requireNotNull(secure)
        repeat(40) { if(evaluate(secureView,"document.querySelector('.wallet-manager') !== null")=="true") return@repeat;Thread.sleep(100) }
        assertEquals("wallet_horizontal_overflow","true",evaluate(secureView,"document.documentElement.scrollWidth <= innerWidth + 1"))
        assertTrue(wallet.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE != 0)
        instrumentation.runOnMainSync { wallet.finish();activity.finish() }
    }
}
