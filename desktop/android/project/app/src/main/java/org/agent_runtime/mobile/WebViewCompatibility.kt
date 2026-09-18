package org.agent_runtime.mobile

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.net.Uri
import android.webkit.WebView

/** Old phones can update their web engine without changing Android itself. */
object WebViewCompatibility {
    fun check(activity: Activity) {
        val major=WebView.getCurrentWebViewPackage()?.versionName?.substringBefore('.')?.toIntOrNull() ?: 0
        if(major >= 111) return
        AlertDialog.Builder(activity)
            .setTitle(R.string.webview_update_title)
            .setMessage(R.string.webview_update_message)
            .setCancelable(false)
            .setPositiveButton(R.string.webview_update_action) { _,_ ->
                activity.startActivity(Intent(Intent.ACTION_VIEW,Uri.parse("https://play.google.com/store/apps/details?id=com.google.android.webview")))
                activity.finish()
            }
            .setNegativeButton(R.string.close_app) { _,_ -> activity.finish() }
            .show()
    }
}
