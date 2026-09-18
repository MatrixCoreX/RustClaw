# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile
-keep class org.agent_runtime.mobile.NativeBridge { *; }
-keep class org.agent_runtime.mobile.WalletActivity { *; }
-keep class org.agent_runtime.mobile.CompanionActivity { *; }
-keep class org.agent_runtime.mobile.VaultService { *; }
# The separately packaged instrumentation runner shares Kotlin with the release
# app. Preserve its runtime entry points across independent R8 optimization.
-keep class kotlin.jvm.internal.Intrinsics { *; }
-keep class androidx.tracing.** { *; }
