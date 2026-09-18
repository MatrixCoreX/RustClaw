package org.agent_runtime.mobile

import android.os.ParcelFileDescriptor
import androidx.core.content.FileProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Test
import org.junit.Assert.*
import org.junit.runner.RunWith
import org.json.JSONObject
import org.json.JSONArray
import java.io.*
import java.util.UUID

@RunWith(AndroidJUnit4::class)
class WalletRoundTripTest {
    @Test fun encryptedRecoveryAndBoundTransactionSignatures() {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        val context=instrumentation.targetContext
        NativeBridge.initialize(context)
        var vault=File(context.cacheDir,"roundtrip-"+UUID.randomUUID())
        var channel=NativeVaultChannel(context)
        fun call(request:JSONObject)=channel.call(request)
        fun request(operation:String)=JSONObject().put("operation",operation)
        fun ok(value:JSONObject):Any { assertTrue(value.toString(),value.has("Ok"));return value.get("Ok") }
        val password="test-only-vault-password"
        val backupPassword="Jasper!flume7-Pebble4-Orbit9-velvet"
        val signatures=JSONArray()
        try {
            ok(call(request("open").put("directory",vault.path)))
            ok(call(request("initialize").put("password",password)))
            var account=ok(call(request("create").put("name","Synthetic mobile account"))) as JSONObject
            val recipient=ok(call(request("create").put("name","Synthetic recipient"))) as JSONObject
            assertNotEquals(account.getString("public_key"),recipient.getString("public_key"))
            val directory=File(context.cacheDir,"document-transfer").apply { mkdirs() }
            val stage=File(directory,UUID.randomUUID().toString())
            val exported=File(context.cacheDir,"verified-mobile.backup.json").apply { createNewFile() }
            val uri=FileProvider.getUriForFile(context,context.packageName+".fileprovider",exported)
            File(stage.path+".destination").writeText(uri.toString())
            ok(call(request("backup").put("id",account.getString("id")).put("vault_password",password)
                .put("password",backupPassword).put("path",stage.path)))
            assertTrue(exported.length()>100);assertFalse(stage.exists())
            val backup=JSONObject(exported.readText());assertEquals("asset-account-backup-v2",backup.getString("format"))
            assertFalse((ok(call(request("status"))) as JSONObject).getBoolean("unlocked"))
            channel.close()
            channel=NativeVaultChannel(context)
            vault=File(context.cacheDir,"recovered-"+UUID.randomUUID())
            ok(call(request("open").put("directory",vault.path)))
            ok(call(request("initialize").put("password",password)))
            val restored=ok(call(request("restore").put("password",backupPassword).put("path",exported.path).put("name","Restored"))) as JSONObject
            assertEquals(account.getString("public_key"),restored.getString("public_key"))
            account=restored
            for((file,secret) in listOf("wallet-linux-v1.json" to "aaaaaaaaaaaa","wallet-linux-v2.json" to backupPassword)) {
                val source=File(context.cacheDir,file)
                instrumentation.context.assets.open(file).use { source.outputStream().use(it::copyTo) }
                val imported=ok(call(request("restore").put("password",secret).put("path",source.path).put("name",file))) as JSONObject
                assertEquals(JSONObject(source.readText()).getString("public_key"),imported.getString("public_key"))
            }
            for(service in listOf("assets","bancor")) {
                val action=if(service=="assets") "transfer" else "bancor_trade"
                val cap=JSONObject().put("schema_version",1).put("protocol","asset_owner_v1").put("ledger_id","mobile-fixture")
                    .put("node_url","https://ledger.example.test").put("service",service).put("actions",JSONArray().put(action))
                val intent=if(service=="assets") JSONObject().put("kind","transfer").put("asset","AIC").put("amount_units","100000000")
                    .put("recipient",recipient.getString("public_key")).put("memo","fixture only").put("max_fee_bps",0)
                else JSONObject().put("kind","bancor_trade").put("side","buy").put("input_units","100000000").put("slippage_bps",100).put("max_fee_bps",0)
                val terms=JSONObject(intent.toString()).put("fee_units","0")
                if(service=="bancor") terms.put("quoted_output_units","100000000").put("min_output_units","99000000")
                val payload=JSONObject().put("schema_version",1).put("protocol","asset_owner_v1").put("ledger_id","mobile-fixture")
                    .put("node_url","https://ledger.example.test").put("service",service).put("account",account.getString("public_key"))
                    .put("operation_id",UUID.randomUUID().toString()).put("challenge_id",UUID.randomUUID().toString())
                    .put("nonce","01".repeat(32)).put("expires_at_unix",System.currentTimeMillis()/1000+120).put("terms",terms)
                val wrong=JSONObject(payload.toString()).put("account",recipient.getString("public_key"))
                assertTrue(call(request("sign").put("id",account.getString("id")).put("password",password)
                    .put("payload",wrong.toString()).put("cap",cap).put("intent",intent)).has("Err"))
                val signature=ok(call(request("sign").put("id",account.getString("id")).put("password",password)
                    .put("payload",payload.toString()).put("cap",cap).put("intent",intent))) as String
                assertEquals(128,signature.length)
                signatures.put(JSONObject().put("public_key",account.getString("public_key")).put("payload",payload.toString()).put("signature",signature))
                assertFalse((ok(call(request("status"))) as JSONObject).getBoolean("unlocked"))
            }
            val document=File(vault,"vault-v1.json").readText()
            assertEquals(2,JSONObject(document).getInt("version"));assertFalse(document.contains(password))
            File(context.getExternalFilesDir(null),"wallet-signatures.json").writeText(signatures.toString())
        } finally { channel.close() }
    }
}
