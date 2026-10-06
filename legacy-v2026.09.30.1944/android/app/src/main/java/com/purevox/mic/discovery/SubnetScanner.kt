/*
 * PureVox — AI 麦克风降噪工具
 * Copyright (C) 2024-2026 a2heng <752848283@qq.com>
 *
# PureVox is licensed under the GNU General Public License v3.0 or
# later (GPL-3.0-or-later).  See LICENSE for details.
# 
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
# 
# The built-in AI models are NOT covered by the GPL; they are the
# property of a2heng and may only be used with PureVox under
# authorization.  See MODEL-LICENSE.md for details.
# 
# SPDX-License-Identifier: GPL-3.0-or-later
 */

package com.purevox.mic.discovery

import okhttp3.OkHttpClient
import okhttp3.Request
import java.net.Inet4Address
import java.net.NetworkInterface
import java.security.SecureRandom
import java.security.cert.X509Certificate
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import javax.net.ssl.SSLContext
import javax.net.ssl.TrustManager
import javax.net.ssl.X509TrustManager

class SubnetScanner(private val port: Int = 59123) {
    var onServerFound: ((DiscoveredServer) -> Unit)? = null
    var onScanComplete: (() -> Unit)? = null

    private val shouldStop = AtomicBoolean(false)
    private val running = AtomicBoolean(false)

    fun start() {
        if (running.getAndSet(true)) return
        shouldStop.set(false)

        val candidates = buildCandidates()
        if (candidates.isEmpty()) {
            onScanComplete?.invoke()
            running.set(false)
            return
        }

        val found = AtomicBoolean(false)
        val done = AtomicInteger(0)
        val total = candidates.size
        val trustAll = buildTrustAllCerts()

        for (ip in candidates) {
            if (shouldStop.get()) break
            Thread {
                tryScan(ip, trustAll)
                if (done.incrementAndGet() >= total) {
                    running.set(false)
                    if (!found.get()) onScanComplete?.invoke()
                }
            }.apply {
                isDaemon = true
                start()
            }
        }
    }

    fun stop() {
        shouldStop.set(true)
        running.set(false)
    }

    private fun buildCandidates(): List<String> {
        val interfaces = NetworkInterface.getNetworkInterfaces() ?: return emptyList()
        val ips = mutableSetOf<String>()
        val seenSubnets = mutableSetOf<String>()

        while (interfaces.hasMoreElements()) {
            val iface = interfaces.nextElement()
            if (iface.isLoopback || !iface.isUp) continue
            for (addr in iface.inetAddresses) {
                if (addr !is Inet4Address || addr.isLoopbackAddress || addr.isLinkLocalAddress) continue
                val bytes = addr.address ?: continue
                val prefix = bytes[0].toInt() and 0xFF
                // 只扫描私有地址段
                if (prefix != 10 && prefix != 172 && prefix != 192) continue
                val subnet = "${bytes[0]}.${bytes[1]}.${bytes[2]}"
                if (subnet in seenSubnets) continue
                seenSubnets.add(subnet)
                val ownLast = bytes[3].toInt() and 0xFF
                for (i in 1..254) {
                    if (i == ownLast) continue
                    ips.add("$subnet.$i")
                }
            }
        }
        return ips.toList()
    }

    private fun tryScan(ip: String, trustAll: Array<TrustManager>) {
        if (shouldStop.get()) return
        try {
            val ssl = SSLContext.getInstance("TLS").apply {
                init(null, trustAll, SecureRandom())
            }
            val client = OkHttpClient.Builder()
                .sslSocketFactory(ssl.socketFactory, trustAll[0] as X509TrustManager)
                .hostnameVerifier { _, _ -> true }
                .connectTimeout(300, java.util.concurrent.TimeUnit.MILLISECONDS)
                .readTimeout(200, java.util.concurrent.TimeUnit.MILLISECONDS)
                .build()
            val request = Request.Builder()
                .url("https://$ip:$port/api/status")
                .build()
            client.newCall(request).execute().use { response ->
                if (response.isSuccessful) {
                    val body = response.body?.string()
                    if (body?.contains("PureVox") == true) {
                        if (!shouldStop.getAndSet(true)) {
                            onServerFound?.invoke(DiscoveredServer(ip, port, "PureVox"))
                        }
                    }
                }
            }
        } catch (_: Exception) {
        }
    }

    private fun buildTrustAllCerts(): Array<TrustManager> {
        return arrayOf(object : X509TrustManager {
            override fun checkClientTrusted(c: Array<out X509Certificate>?, p: String?) {}
            override fun checkServerTrusted(c: Array<out X509Certificate>?, p: String?) {}
            override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
        })
    }
}