package org.agent_runtime.mobile

import android.content.Intent
import android.content.res.Configuration
import android.os.Bundle
import android.view.WindowManager
import android.view.View
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

open class MainActivity : TauriActivity() {
    private var contentView: WebView? = null
    override val handleBackNavigation: Boolean = false
    override fun onWebViewCreate(webView: WebView) {
        super.onWebViewCreate(webView)
        contentView=webView
        webView.settings.textZoom=(resources.configuration.fontScale*100).toInt().coerceIn(80,200)
        webView.importantForAutofill=View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
        webView.settings.allowFileAccess=false
        WebViewCompatibility.check(this)
        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                webView.evaluateJavascript("window.dispatchEvent(new Event('client-back',{cancelable:true}))") { unhandled ->
                    if (unhandled == "false") return@evaluateJavascript
                    if (webView.canGoBack()) webView.goBack()
                    else if (this@MainActivity.javaClass == MainActivity::class.java) moveTaskToBack(true)
                    else finish()
                }
            }
        })
    }
    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        contentView?.settings?.textZoom=(newConfig.fontScale*100).toInt().coerceIn(80,200)
    }
    override fun onCreate(savedInstanceState: Bundle?) {
        NativeBridge.initialize(this)
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        // Private account screens and recents snapshots must never expose credentials.
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        ViewCompat.setOnApplyWindowInsetsListener(window.decorView) { view, insets ->
            val bars=insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
            val ime=insets.getInsets(WindowInsetsCompat.Type.ime())
            view.setPadding(bars.left,bars.top,bars.right,maxOf(bars.bottom,ime.bottom))
            WindowInsetsCompat.CONSUMED
        }
    }
    override fun onResume() { super.onResume(); Documents.attach(this) }
    override fun onActivityResult(requestCode: Int,resultCode: Int,data: Intent?) {
        if (!Documents.result(requestCode,resultCode,data)) super.onActivityResult(requestCode,resultCode,data)
    }
}
class WalletActivity : MainActivity()
class CompanionActivity : MainActivity()
