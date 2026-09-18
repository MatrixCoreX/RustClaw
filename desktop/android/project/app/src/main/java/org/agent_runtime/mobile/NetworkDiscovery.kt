package org.agent_runtime.mobile

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import org.json.JSONArray
import org.json.JSONObject
import java.net.Inet4Address
import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

object NetworkDiscovery {
    fun networks(context: Context): String {
        val manager=context.getSystemService(ConnectivityManager::class.java)
        val network=manager.activeNetwork ?: return "[]"
        val caps=manager.getNetworkCapabilities(network) ?: return "[]"
        if(caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN) ||
            !(caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) || caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET))) return "[]"
        val addresses=manager.getLinkProperties(network)?.linkAddresses ?: return "[]"
        val out=JSONArray()
        addresses.filter { it.address is Inet4Address && it.address.isSiteLocalAddress && it.prefixLength in 1..32 }.forEach {
            val mask=(0xffffffffL shl (32-it.prefixLength)) and 0xffffffffL
            val text=(3 downTo 0).joinToString(".") { n -> ((mask shr(n*8)) and 255).toString() }
            out.put(JSONArray().put(it.address.hostAddress).put(text))
        }
        return out.toString()
    }
    fun mdns(context: Context): String {
        if(networks(context)=="[]") return "[]"
        val manager=context.getSystemService(NsdManager::class.java)
        val records=Collections.synchronizedList(mutableListOf<JSONObject>())
        val stop=CountDownLatch(1)
        val pending=Collections.synchronizedList(mutableListOf<NsdServiceInfo>())
        val listener=object:NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(type:String) {}
            override fun onDiscoveryStopped(type:String) { stop.countDown() }
            override fun onStartDiscoveryFailed(type:String,code:Int) { stop.countDown() }
            override fun onStopDiscoveryFailed(type:String,code:Int) { stop.countDown() }
            override fun onServiceLost(info:NsdServiceInfo) {}
            override fun onServiceFound(info:NsdServiceInfo) { if(pending.size<32) pending.add(info) }
        }
        manager.discoverServices("_agent-runtime._tcp.",NsdManager.PROTOCOL_DNS_SD,listener)
        try {
            val end=System.nanoTime()+TimeUnit.SECONDS.toNanos(4)
            while(System.nanoTime()<end) {
                val info=synchronized(pending) { if(pending.isEmpty()) null else pending.removeAt(0) }
                if(info==null) { Thread.sleep(80); continue }
                val done=CountDownLatch(1)
                manager.resolveService(info,object:NsdManager.ResolveListener {
                    override fun onResolveFailed(info:NsdServiceInfo,code:Int) { done.countDown() }
                    override fun onServiceResolved(value:NsdServiceInfo) {
                        try {
                            val host=value.host
                            val attrs=value.attributes.mapValues { String(it.value,Charsets.UTF_8) }
                            if(host is Inet4Address && host.isSiteLocalAddress && value.port in 1..65535 &&
                                (attrs["scheme"]==null || attrs["scheme"]=="https") && (attrs["api"]==null || attrs["api"]=="webd-v1")) {
                                val ip=host.hostAddress!!
                                records.add(JSONObject().put("name",value.serviceName).put("address","https://$ip:${value.port}")
                                    .put("kind","https").put("port",value.port).put("source","mdns").put("private_ca",attrs["tls"]=="private_ca")
                                    .put("verified",false).put("ips",JSONArray().put(ip)))
                            }
                        } finally { done.countDown() }
                    }
                })
                done.await(800,TimeUnit.MILLISECONDS)
            }
        } finally { manager.stopServiceDiscovery(listener) }
        return synchronized(records) { JSONArray(records.toList()).toString() }
    }
}
