// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// The built-in AI models are NOT covered by the GPL; they are the
// property of a2heng and may only be used with PureVox under
// authorization.  See MODEL-LICENSE.md for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

package com.purevox.mic.net

import android.annotation.SuppressLint
import java.security.SecureRandom
import java.security.cert.X509Certificate
import javax.net.ssl.HostnameVerifier
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSession
import javax.net.ssl.SSLSocketFactory
import javax.net.ssl.X509TrustManager
import javax.net.ssl.HttpsURLConnection

/**
 * 局域网自签证书的放行范围（NETWORK.md §4.1.1）。
 *
 * **为什么需要它**：电脑端的服务是 HTTPS，而证书是每台机器自己签的（`~/.purevox/ca/`），
 * 手机系统信任库里没有。若要求用户先把 CA 装进系统，纯本地工具就多了一步非做不可的
 * 手续，而它换来的是什么？这套 TLS 的**唯一**目的是让**浏览器**拿到安全上下文
 * （`getUserMedia` 的硬性前提），局域网内本来就没有鉴权（协议层已明确记录这个取舍）。
 *
 * **为什么仍然要限定范围**：不能用「无条件信任所有证书」——那是纯粹的安全反模式，会让
 * 这个 app 在任何域���上都丧失 TLS 的意义（连 `https://example.com` 都能被中间人）。
 * 所以只对**私有网段**放行：这些地址在公网不可路由，攻击者要冒充必须先在同一个局域网里，
 * 威胁模型与「局域网无鉴权」完全一致。公网地址与域名一律走系统默认校验。
 *
 * 想彻底不要这个放行：把 [isPrivateHost] 改成永远 `false`，然后在手机上装电脑端的 CA
 * （`https://<电脑IP>:59123/ca-cert` 下载后走系统「安装证书」）。
 */
object LocalTls {

    /**
     * 地址是不是**私有网段 / 环回 / 链路本地**。
     *
     * 覆盖：IPv4 的 `10/8`、`172.16/12`、`192.168/16`、`127/8`、`169.254/16`；
     * IPv6 的 `::1`、`fe80::/10`（链路本地）、`fc00::/7`（唯一本地）。
     * 用字符串前缀而不是 `InetAddress` 的类型判断：主机名可能是域名（此时不该放行），
     * 而我们要判断的是「这个**字面地址**是不是内网」。
     */
    fun isPrivateHost(host: String): Boolean {
        val h = host.trim().removeSurrounding("[", "]").lowercase()
        if (h.isEmpty()) return false
        if (h == "localhost") return true
        if (h == "::1" || h == "0:0:0:0:0:0:0:1") return true
        // IPv6 链路本地 fe80::/10 与唯一本地 fc00::/7
        if (h.startsWith("fe8") || h.startsWith("fe9") || h.startsWith("fea") || h.startsWith("feb")) return true
        if (h.startsWith("fc") || h.startsWith("fd")) return true
        if (!h.contains(':')) {
            val p = h.split(".")
            if (p.size != 4) return false
            val n = p.map { it.toIntOrNull() ?: return false }
            return when {
                n[0] == 127 -> true                       // 环回
                n[0] == 10 -> true                        // 10/8
                n[0] == 172 && n[1] in 16..31 -> true      // 172.16/12
                n[0] == 192 && n[1] == 168 -> true         // 192.168/16
                n[0] == 169 && n[1] == 254 -> true         // 链路本地
                else -> false
            }
        }
        return false
    }

    /**
     * 本次要连的主机。`X509TrustManager` **拿不到主机名**（它只看证书链），所以要知道
     * 「这条链是不是连私有地址时拿到的」，只能由调用方在发起连接前记下来。
     *
     * 同一时刻只连一台电脑（这也是产品形态：局域网里就一台），所以一个字段够用；用
     * `@Volatile` 是因为它跨线程（UI 线程写、OkHttp 线程读）。
     */
    @Volatile
    private var targetHost: String = ""

    /** 发起连接**前**记下目标主机，证书校验时据此决定放不放行。 */
    fun setTarget(host: String) {
        targetHost = host
    }

    /** 本次连接是否享受「私有地址放行」。 */
    fun relaxedForTarget(): Boolean = isPrivateHost(targetHost)

    /**
     * 只在**私有地址**上信任任意证书；其余地址用系统默认信任链。
     *
     * 两条路都保留，所以公网证书过期 / 主机名不匹配之类的问题仍然会照常报错，不会被
     * 这段代码悄悄吞掉。
     */
    @SuppressLint("CustomX509TrustManager")
    fun trustManager(): X509TrustManager = object : X509TrustManager {
        private val platform = defaultTrustManager()

        @SuppressLint("TrustAllX509TrustManager")
        override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) =
            platform.checkClientTrusted(chain, authType)

        /**
         * 私有地址 → 放行自签证书；其余 → 走系统校验。
         *
         * 注意这里**不能**无条件 `return`：那会让本 app 在任何域名上都丧失 TLS 的意义。
         */
        @SuppressLint("TrustAllX509TrustManager")
        override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {
            if (isPrivateHost(targetHost)) return   // 私有网段：电脑端自签，放行
            platform.checkServerTrusted(chain, authType)
        }

        override fun getAcceptedIssuers(): Array<X509Certificate> = platform.acceptedIssuers
    }

    /** 系统默认信任链（拿来做「严格」那一路）。 */
    private fun defaultTrustManager(): X509TrustManager {
        val tmf = javax.net.ssl.TrustManagerFactory.getInstance(javax.net.ssl.TrustManagerFactory.getDefaultAlgorithm())
        tmf.init(null as java.security.KeyStore?)
        return tmf.trustManagers.first { it is X509TrustManager } as X509TrustManager
    }

    fun socketFactory(): SSLSocketFactory =
        SSLContext.getInstance("TLS").apply {
            init(null, arrayOf(trustManager()), SecureRandom())
        }.socketFactory

    /**
     * 主机名校验同样只对私有地址放行。私有地址的证书 SAN 里带的是 IP（服务端按当前网卡
     * 集合签的），而这里传的可能是域名形式，所以私有地址一律不校验名字。
     */
    fun hostnameVerifier(): HostnameVerifier = HostnameVerifier { host, session ->
        if (isPrivateHost(host)) true
        else HttpsURLConnection.getDefaultHostnameVerifier().verify(host, session)
    }
}
