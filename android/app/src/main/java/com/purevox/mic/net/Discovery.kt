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

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import android.util.Log

/**
 * 局域网自动发现（NETWORK.md §4.1.1）。
 *
 * 用系统自带的 `NsdManager`（零依赖、零权限），找电脑端广播的 `_purevox._tcp.local.`。
 *
 * **为什么手机不需要「选电脑」的下拉**：局域网里就一台电脑，选了也没得选。用户想连的是
 * **正在跑服务的那台**，所以发现到就直接用。发现失败才给手动输入兜底（换 Wi-Fi、组播被
 * 交换机挡了、AP 隔离）。
 *
 * **「重新发现」与「重新连接」是两件事**（界面上是两个按钮）：换网络才需要重新发现；只是
 * 掉线、服务还在时，重新发现要等好几秒而重新连接立刻就好。把它们分开是让用户在正确的
 * 时候做正确的事。
 *
 * **发现要常开着，不是「找到就停」**：服务停了再启动时要能自动跟上，所以发现回调一直挂着，
 * 每次解析到就回调一次（上层自己去重）。
 */
class Discovery(
    context: Context,
    private val onFound: (host: String, port: Int, name: String, proto: Int?) -> Unit,
    private val onLost: (String) -> Unit,
    private val onError: (String) -> Unit,
) {
    private val app = context.applicationContext
    private val nsd: NsdManager? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.JELLY_BEAN_MR1) {
            app.getSystemService(Context.NSD_SERVICE) as? NsdManager
        } else {
            null
        }

    private var listener: NsdManager.DiscoveryListener? = null
    private var resolving: NsdManager.ResolveListener? = null
    private var running = false

    /** 本机地址（发现用不上，但排错时能看到用户当前在什么网段）。 */
    val available: Boolean get() = nsd != null

    /**
     * 开始发现。**幂等**：已经在跑就直接返回，不重复注册监听（`NsdManager` 重复
     * `discoverServices` 会抛 `IllegalArgumentException`）。
     */
    fun start() {
        val m = nsd ?: run {
            onError("这台系统的 NSD 服务不可用，请手动填地址")
            return
        }
        if (running) return
        val l = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(serviceType: String) {
                Log.i(TAG, "开始发现 $serviceType")
            }

            override fun onServiceFound(info: NsdServiceInfo) {
                val name = info.serviceName ?: return
                // 实例名形如 `PureVox 192.168.1.5`，末尾那段就是 IP，直接取，省一次解析；
                // 取不到（用户改了实例名）再走系统解析
                val tail = name.substringAfterLast(' ', "").trim()
                if (tail.isNotEmpty() && tail.all { it.isDigit() || it == '.' } && tail.count { it == '.' } == 3) {
                    onFound(tail, info.port, name, null)
                    return
                }
                resolve(m, info, name)
            }

            override fun onServiceLost(info: NsdServiceInfo) {
                onLost(info.serviceName ?: "")
            }

            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                running = false
                onError("开始发现失败（$errorCode），请手动填地址")
            }

            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {
                Log.w(TAG, "停止发现失败 $errorCode")
            }

            override fun onDiscoveryStopped(serviceType: String) {
                Log.i(TAG, "停止发现 $serviceType")
            }
        }
        try {
            m.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, l)
            listener = l
            running = true
        } catch (e: IllegalArgumentException) {
            // 系统已经有一个同类型监听了（多实例 / 上次没停干净）：先停再起
            Log.w(TAG, "重复发现，先停一次", e)
            runCatching { m.stopServiceDiscovery(l) }
            runCatching { m.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, l) }
                .onFailure { running = false; onError("发现启动失败：${e.message}") }
        } catch (e: Exception) {
            running = false
            onError("发现启动失败：${e.message}")
        }
    }

    /** 解析实例名里取不到 IP 的情况（用户自定义过实例名）。 */
    private fun resolve(m: NsdManager, info: NsdServiceInfo, name: String) {
        val rl = object : NsdManager.ResolveListener {
            override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) {
                Log.w(TAG, "解析 $name 失败 $errorCode")
            }

            override fun onServiceResolved(info: NsdServiceInfo) {
                val host = hostOf(info) ?: return
                val proto = info.attributes["proto"]
                    ?.toString(Charsets.UTF_8)
                    ?.trim()
                    ?.toIntOrNull()
                onFound(host, info.port, name, proto)
            }
        }
        resolving = rl
        @Suppress("DEPRECATION")
        runCatching { m.resolveService(info, rl) }
            .onFailure { resolving = null }
    }

    /**
     * 从解析结果里取地址。
     *
     * **不用 `InetAddress.getHostAddress()` 的裸形式**：Android 7 起它返回的是**去掉了
     * 前导零的压缩写法**（例如 `fe80::1` 而不是 `fe80:0000:...:0001`），而我们连的是
     * 局域网 IPv4，直接取 `Inet4Address` 的四个字节拼点分十进制最稳，不受任何格式化规则影响。
     */
    private fun hostOf(info: NsdServiceInfo): String? {
        val a = info.host ?: return null
        val v4 = a as? java.net.Inet4Address ?: return a.hostAddress
        val b = v4.address
        // 不用字符串模板拼：`${b[i] & 0xFF}` 里同时有下标与位运算，模板解析容易出问题，
        // 而这里就是把四个 0..255 变成点分十进制，循环最直白
        val sb = StringBuilder(15)
        for (i in 0 until 4) {
            if (i > 0) sb.append('.')
            sb.append(b[i].toInt() and 0xFF)
        }
        return sb.toString()
    }

    /** 停止发现。**必须停**，否则 `NsdManager` 的监听会一直占着（耗电，且下次启动会撞重复注册）。 */
    fun stop() {
        val m = nsd ?: return
        listener?.let { runCatching { m.stopServiceDiscovery(it) } }
        listener = null
        resolving = null
        running = false
    }

    companion object {
        private const val TAG = "PureVoxDiscovery"

        /** 必须与电脑端 `net::mdns::SERVICE_TYPE` 一致（DNS-SD 按 RFC 6763 约定小写）。 */
        const val SERVICE_TYPE = "_purevox._tcp."
    }
}
