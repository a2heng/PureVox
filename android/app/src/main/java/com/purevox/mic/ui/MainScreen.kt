/*
 * PureVox — AI 麦克风降噪工具
 * Copyright (C) 2024-2026 a2heng <752848283@qq.com>
 *
 * PureVox is licensed under the GNU General Public License v3.0 or
 * later (GPL-3.0-or-later).  See LICENSE for details.
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * The built-in AI models are NOT covered by the GPL; they are the
 * property of a2heng and may only be used with PureVox under
 * authorization.  See MODEL-LICENSE.md for details.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

package com.purevox.mic.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsBottomHeight
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import com.itos.coui.layout.Divider
import com.itos.coui.layout.Group
import com.itos.coui.layout.Item
import com.itos.coui.slider.Slider
import com.itos.coui.switch.Switch
import com.itos.coui.theme.CouiColorScheme
import com.itos.coui.theme.CouiTheme
import com.purevox.mic.MainActivity
import com.purevox.mic.R
import com.purevox.mic.audio.MicSources
import com.purevox.mic.update.UpdateChecker

/** 右侧 hero 上方的品牌色：与旧 XML 的头图渐变一致（深蓝 → 亮一点的蓝）。 */
private val HeroTop = Color(0xFF132C50)
private val HeroBottom = Color(0xFF2C5C9F)
/** hero 上的次级文字（副标题 / 目标地址）。 */
private val HeroMuted = Color(0xFFB9D2F5)
/** 就绪状态的大字（偏绿的亮色，与深蓝底对比够）。 */
private val HeroReady = Color(0xFFC7F7DA)

/**
 * 界面根：COUI 主题 + Material 配色桥接。
 *
 * 为什么还要包一层 [MaterialTheme]：COUI 没有输入框 / 按钮，这两类直接用 Material 3；
 * 把 COUI 的色板映射进 Material 的 `colorScheme`，才不会出现 M3 默认的紫色。
 */
@Composable
internal fun PureVoxApp(activity: MainActivity) {
    // 订阅 2 Hz 的状态刷新：updateStatus() 末尾会 uiTick++，Compose 根跟着重算一遍
    activity.uiTick

    CouiTheme(darkTheme = isSystemInDarkTheme()) {
        val scheme = CouiTheme.colorScheme
        MaterialTheme(colorScheme = scheme.toMaterialColorScheme()) {
            Box(Modifier.fillMaxSize().background(scheme.background)) {
                HomeScreen(activity)
                if (activity.settingsOpen) SettingsDialog(activity)
            }
        }
    }
}

/** COUI 色板 → Material 3 色板；只映射实际用到的角色，不引入 coui-material3 初始化。 */
@Composable
private fun CouiColorScheme.toMaterialColorScheme(): ColorScheme {
    val dark = CouiTheme.isDark
    val base = if (dark) darkColorScheme() else lightColorScheme()
    return base.copy(
        primary = accent,
        onPrimary = onAccent,
        primaryContainer = accent.copy(alpha = if (dark) 0.32f else 0.12f),
        onPrimaryContainer = onSurface,
        secondary = accent,
        onSecondary = onAccent,
        background = background,
        onBackground = onSurface,
        surface = surface,
        onSurface = onSurface,
        surfaceVariant = container,
        onSurfaceVariant = onSurfaceVariant,
        outline = outlineVariant,
        outlineVariant = outlineVariant,
        error = Color(0xFFC62828),
        onError = Color.White,
    )
}

// ------------------------------------------------------------------ 主页

@Composable
private fun HomeScreen(activity: MainActivity) {
    val scheme = CouiTheme.colorScheme
    Column(
        Modifier
            .fillMaxSize()
            .imePadding()
            .verticalScroll(rememberScrollState()),
    ) {
        Hero(activity)

        // 三个开关 = 三个「现在要不要发生」，整行可点，不是只有开关本体
        Group {
            SwitchRow(
                text = stringResource(R.string.sw_connect),
                checked = activity.connectOn,
                enabled = !activity.connecting,
                onCheckedChange = { activity.setConnect(it) },
            )
            Divider()
            SwitchRow(
                text = stringResource(R.string.sw_mic),
                checked = activity.micOn,
                enabled = activity.ready,
                onCheckedChange = { activity.setMic(it) },
            )
            Divider()
            SwitchRow(
                text = stringResource(R.string.sw_speaker),
                checked = activity.spkOn,
                enabled = activity.ready,
                onCheckedChange = { activity.setSpeaker(it) },
            )
        }

        TargetSection(activity, scheme)
        AudioSection(activity, scheme)

        Text(
            text = stringResource(R.string.footer_hint),
            modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 16.dp),
            fontSize = 11.sp,
            lineHeight = 15.sp,
            color = scheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.windowInsetsBottomHeight(WindowInsets.navigationBars))
    }
}

/** 头图：品牌字标 + 状态点 + 大字状态 + 目标地址（状态栏区域由渐变铺满）。 */
@Composable
private fun Hero(activity: MainActivity) {
    val dotColor = when {
        activity.ready -> Color(0xFF7CE3A8)
        activity.connectOn -> Color(0xFFF0C46A)
        else -> Color(0xFFB9C6DA)
    }
    Box(
        Modifier
            .fillMaxWidth()
            // 下沿留圆角、上沿不留（要铺到状态栏后面）；左右贴屏幕边
            .clip(RoundedCornerShape(bottomStart = 24.dp, bottomEnd = 24.dp))
            .background(Brush.verticalGradient(listOf(HeroTop, HeroBottom))),
    ) {
        Column(
            Modifier
                .windowInsetsPadding(WindowInsets.statusBars)
                .padding(start = 20.dp, top = 10.dp, end = 14.dp, bottom = 18.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                // 字标是**品牌**，不进 i18n：P 的包围盒比大写高度高，往上挪 2dp 贴住基线
                Image(
                    painter = painterResource(R.drawable.ic_p_mark),
                    contentDescription = stringResource(R.string.app_name),
                    modifier = Modifier.size(24.dp, 36.dp).offset(y = (-2).dp),
                    contentScale = ContentScale.Fit,
                )
                Text(
                    text = stringResource(R.string.wordmark_tail),
                    modifier = Modifier.padding(start = 2.dp),
                    fontSize = 24.sp,
                    fontWeight = FontWeight.Bold,
                    letterSpacing = 0.5.sp,
                    color = Color.White,
                )
                Text(
                    text = stringResource(R.string.subtitle),
                    modifier = Modifier
                        .weight(1f)
                        .padding(start = 10.dp),
                    fontSize = 11.sp,
                    letterSpacing = 0.2.sp,
                    color = HeroMuted,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                IconButton(
                    onClick = { activity.openSettings() },
                    modifier = Modifier
                        .padding(start = 6.dp)
                        .size(42.dp)
                        .clip(CircleShape)
                        .background(Color.White.copy(alpha = 0.14f)),
                ) {
                    Icon(
                        imageVector = Icons.Filled.Settings,
                        contentDescription = stringResource(R.string.btn_settings),
                        tint = Color.White,
                        modifier = Modifier.size(21.dp),
                    )
                }
            }

            Row(
                modifier = Modifier.padding(top = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(Modifier.size(9.dp).clip(CircleShape).background(dotColor))
                Text(
                    text = activity.connState,
                    modifier = Modifier.padding(start = 9.dp),
                    fontSize = 17.sp,
                    fontWeight = FontWeight.Bold,
                    color = if (activity.ready) HeroReady else Color.White,
                )
            }

            Text(
                text = activity.heroTargetText,
                modifier = Modifier.padding(start = 18.dp, top = 3.dp),
                fontSize = 11.sp,
                fontFamily = FontFamily.Monospace,
                color = HeroMuted,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

/** 一行开关：整行可点（COUI 的 Item 提供行内留白与按压反馈）。 */
@Composable
private fun SwitchRow(
    text: String,
    checked: Boolean,
    enabled: Boolean,
    onCheckedChange: (Boolean) -> Unit,
) {
    val scheme = CouiTheme.colorScheme
    Item(
        onClick = { if (enabled) onCheckedChange(!checked) },
        enabled = enabled,
        // 禁用不只是「点不动」：文字跟着变淡，才知道是状态不允许（未就绪 / 正在连接）
        title = {
            Text(
                text = text,
                fontSize = 16.sp,
                color = if (enabled) scheme.onSurface else scheme.onSurface.copy(alpha = 0.4f),
            )
        },
        trailing = {
            Switch(
                checked = checked,
                onCheckedChange = { if (enabled) onCheckedChange(it) },
                enabled = enabled,
            )
        },
    )
}

/** 连接目标：两个输入框 + 扫码 + 发现 / 重连。 */
@Composable
private fun TargetSection(activity: MainActivity, scheme: CouiColorScheme) {
    val fieldColors = OutlinedTextFieldDefaults.colors(
        focusedBorderColor = scheme.accent,
        unfocusedBorderColor = scheme.outlineVariant,
        focusedTextColor = scheme.onSurface,
        unfocusedTextColor = scheme.onSurface,
        cursorColor = scheme.accent,
        focusedPlaceholderColor = scheme.onSurfaceVariant,
        unfocusedPlaceholderColor = scheme.onSurfaceVariant,
    )
    Group(title = stringResource(R.string.section_target)) {
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            OutlinedTextField(
                value = activity.hostInput,
                onValueChange = { activity.hostInput = it },
                modifier = Modifier.weight(1f),
                singleLine = true,
                shape = RoundedCornerShape(12.dp),
                placeholder = { Text(stringResource(R.string.hint_host), fontSize = 13.sp) },
                textStyle = MaterialTheme.typography.bodyMedium.copy(fontSize = 14.sp),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                colors = fieldColors,
            )
            OutlinedTextField(
                value = activity.portInput,
                onValueChange = { raw ->
                    // 端口只收数字、最多 5 位；非法值在 readTarget() 里回退到默认端口
                    activity.portInput = raw.filter { it in '0'..'9' }.take(5)
                },
                modifier = Modifier.width(92.dp),
                singleLine = true,
                shape = RoundedCornerShape(12.dp),
                placeholder = { Text(stringResource(R.string.hint_port), fontSize = 13.sp) },
                textStyle = MaterialTheme.typography.bodyMedium.copy(fontSize = 14.sp),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                colors = fieldColors,
            )
            IconButton(onClick = { activity.startScan() }) {
                Icon(
                    imageVector = Icons.Filled.QrCodeScanner,
                    contentDescription = stringResource(R.string.btn_scan),
                    tint = scheme.onSurfaceVariant,
                    modifier = Modifier.size(24.dp),
                )
            }
        }
        Row(
            Modifier.fillMaxWidth().padding(end = 8.dp),
            horizontalArrangement = Arrangement.End,
        ) {
            TextButton(onClick = { activity.rediscover() }) {
                Text(stringResource(R.string.btn_rediscover), fontSize = 13.sp)
            }
            TextButton(onClick = { activity.reconnect() }) {
                Text(stringResource(R.string.btn_reconnect), fontSize = 13.sp)
            }
        }
    }
}

/** 抖动缓冲 + 麦克风 API：属性类参数，不折叠、不藏进设置。 */
@Composable
private fun AudioSection(activity: MainActivity, scheme: CouiColorScheme) {
    Group {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)) {
            Text(
                text = stringResource(R.string.ring_label, activity.ringMs / 10, activity.ringMs),
                fontSize = 12.sp,
                color = scheme.onSurfaceVariant,
            )
            // 拖动即生效（不重建会话、不重连），并同步给电脑
            Slider(
                modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
                value = activity.ringMs.toFloat(),
                onValueChange = { activity.setRingTarget(it.toInt()) },
                valueRange = 10f..200f,
                steps = 18,
            )
        }
        Divider()
        MicSourceRow(activity, scheme)
        Text(
            text = stringResource(R.string.mic_source_hint),
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            fontSize = 11.sp,
            lineHeight = 15.sp,
            color = scheme.onSurfaceVariant,
        )
    }
}

/** 麦克风 API：一个可点的行，点开就是完整清单（选中立即生效，推流中会重启 uplink）。 */
@Composable
private fun MicSourceRow(activity: MainActivity, scheme: CouiColorScheme) {
    var menuOpen by remember { mutableStateOf(false) }
    Box {
        Item(
            onClick = { menuOpen = true },
            title = { Text(stringResource(R.string.mic_source_label), fontSize = 16.sp) },
            trailing = {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        text = MicSources.label(activity, activity.micSourceId),
                        modifier = Modifier.widthIn(max = 160.dp),
                        fontSize = 13.sp,
                        color = scheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Icon(
                        imageVector = Icons.Filled.ExpandMore,
                        contentDescription = null,
                        tint = scheme.onSurfaceVariant,
                        modifier = Modifier.padding(start = 4.dp).size(20.dp),
                    )
                }
            },
        )
        DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
            MicSources.ALL.forEach { source ->
                DropdownMenuItem(
                    text = { Text(stringResource(source.labelRes), fontSize = 14.sp) },
                    onClick = {
                        menuOpen = false
                        activity.selectMicSource(source.id)
                    },
                )
            }
        }
    }
}

// ------------------------------------------------------------------ 设置

@Composable
private fun SettingsDialog(activity: MainActivity) {
    val scheme = CouiTheme.colorScheme
    Dialog(
        onDismissRequest = { activity.closeSettings() },
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        // 居中浮现（不是底部弹层）：窗口撑满，卡片自己居中 + 让出系统栏
        Box(
            Modifier
                .fillMaxSize()
                .windowInsetsPadding(WindowInsets.safeDrawing)
                .padding(horizontal = 12.dp, vertical = 24.dp),
            contentAlignment = Alignment.Center,
        ) {
            Column(
                Modifier
                    .fillMaxWidth()
                    .heightIn(max = 640.dp)
                    .clip(RoundedCornerShape(24.dp))
                    .background(scheme.container),
            ) {
                Row(
                    Modifier.fillMaxWidth().padding(start = 20.dp, top = 12.dp, end = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = stringResource(R.string.sheet_title),
                        modifier = Modifier.weight(1f),
                        fontSize = 20.sp,
                        fontWeight = FontWeight.Bold,
                        color = scheme.onSurface,
                    )
                    IconButton(onClick = { activity.closeSettings() }) {
                        Icon(
                            imageVector = Icons.Filled.Close,
                            contentDescription = stringResource(R.string.sheet_close),
                            tint = scheme.onSurfaceVariant,
                        )
                    }
                }
                HorizontalDivider(color = scheme.outlineVariant)
                Column(
                    Modifier
                        .weight(1f, fill = false)
                        .verticalScroll(rememberScrollState())
                        .padding(bottom = 18.dp),
                ) {
                    LanguageSection(activity)
                    KeepaliveSection(activity, scheme)
                    UpdateSection(activity, scheme)
                    AboutSection(activity, scheme)
                    DiagSection(activity, scheme)
                }
            }
        }
    }
}

/** 界面语言：跟随系统 / 中文 / English（选完立即切换并重建 Activity）。 */
@Composable
private fun LanguageSection(activity: MainActivity) {
    val scheme = CouiTheme.colorScheme
    val current = activity.currentLanguageTag()
    val options = listOf(
        "" to stringResource(R.string.lang_system),
        "zh" to stringResource(R.string.lang_zh),
        "en" to stringResource(R.string.lang_en),
    )
    Group(title = stringResource(R.string.section_language)) {
        Row(
            Modifier.fillMaxWidth().padding(12.dp).clip(RoundedCornerShape(12.dp))
                .background(scheme.background),
        ) {
            options.forEach { (tag, label) ->
                val selected = tag == current
                Box(
                    Modifier
                        .weight(1f)
                        .clip(RoundedCornerShape(12.dp))
                        .background(if (selected) scheme.accent else Color.Transparent)
                        .clickable { if (!selected) activity.setLanguage(tag) }
                        .padding(vertical = 9.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = label,
                        fontSize = 13.sp,
                        maxLines = 1,
                        color = if (selected) scheme.onAccent else scheme.onSurface,
                    )
                }
            }
        }
    }
}

@Composable
private fun KeepaliveSection(activity: MainActivity, scheme: CouiColorScheme) {
    Group(title = stringResource(R.string.section_keepalive)) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp)) {
            Text(
                text = stringResource(R.string.keepalive_desc),
                fontSize = 12.sp,
                lineHeight = 17.sp,
                color = scheme.onSurfaceVariant,
            )
            Text(
                text = activity.keepaliveText,
                modifier = Modifier.padding(top = 8.dp),
                fontSize = 12.sp,
                color = scheme.onSurface,
            )
            Row(Modifier.fillMaxWidth().padding(top = 4.dp)) {
                Text(
                    text = activity.notifPermText,
                    modifier = Modifier.weight(1f),
                    fontSize = 11.sp,
                    color = scheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = activity.batteryText,
                    modifier = Modifier.weight(1f),
                    fontSize = 11.sp,
                    textAlign = TextAlign.End,
                    color = scheme.onSurfaceVariant,
                    maxLines = 1,
                )
            }
            Row(
                Modifier.fillMaxWidth().padding(top = 10.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                OutlinedButton(
                    onClick = { activity.openNotifSettings() },
                    modifier = Modifier.weight(1f),
                    shape = RoundedCornerShape(13.dp),
                ) {
                    Text(stringResource(R.string.btn_notif_settings), fontSize = 13.sp, maxLines = 1)
                }
                OutlinedButton(
                    onClick = { activity.requestIgnoreBattery() },
                    modifier = Modifier.weight(1f),
                    shape = RoundedCornerShape(13.dp),
                ) {
                    Text(stringResource(R.string.btn_battery_settings), fontSize = 13.sp, maxLines = 1)
                }
            }
        }
    }
}

@Composable
private fun UpdateSection(activity: MainActivity, scheme: CouiColorScheme) {
    Group(title = stringResource(R.string.section_update)) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp)) {
            Text(
                text = activity.updateText,
                fontSize = 13.sp,
                lineHeight = 18.sp,
                color = scheme.onSurface,
            )
            OutlinedButton(
                onClick = { activity.checkUpdate() },
                modifier = Modifier.fillMaxWidth().padding(top = 10.dp),
                enabled = !activity.updateBusy,
                shape = RoundedCornerShape(13.dp),
            ) {
                Text(stringResource(R.string.btn_check_update), fontSize = 13.sp)
            }
            Row(
                Modifier.fillMaxWidth().padding(top = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Button(
                    onClick = { activity.downloadUpdate() },
                    modifier = Modifier.weight(1f),
                    enabled = !activity.updateBusy && activity.updateRelease != null &&
                        activity.updateApk == null,
                    shape = RoundedCornerShape(13.dp),
                    colors = ButtonDefaults.buttonColors(),
                ) {
                    Text(stringResource(R.string.btn_download), fontSize = 13.sp)
                }
                Button(
                    onClick = { activity.installUpdate() },
                    modifier = Modifier.weight(1f),
                    enabled = !activity.updateBusy && activity.updateApk != null,
                    shape = RoundedCornerShape(13.dp),
                    colors = ButtonDefaults.buttonColors(),
                ) {
                    Text(stringResource(R.string.btn_install), fontSize = 13.sp)
                }
            }
        }
    }
}

@Composable
private fun AboutSection(activity: MainActivity, scheme: CouiColorScheme) {
    Group(title = stringResource(R.string.section_about)) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp)) {
            Text(
                text = stringResource(
                    R.string.about_version,
                    UpdateChecker.installedName(activity),
                    UpdateChecker.installedCode(activity).toString(),
                ),
                fontSize = 12.sp,
                color = scheme.onSurface,
            )
            Text(
                text = stringResource(R.string.about_license),
                modifier = Modifier.padding(top = 5.dp),
                fontSize = 11.sp,
                lineHeight = 16.sp,
                color = scheme.onSurfaceVariant,
            )
        }
    }
}

/** 诊断：常显（出错时用户不必再点开一次），读数与日志都能一键复制。 */
@Composable
private fun DiagSection(activity: MainActivity, scheme: CouiColorScheme) {
    // 日志是普通集合：读一次追加计数，日志文本才会跟着新行重组
    @Suppress("UNUSED_VARIABLE") val logTick = activity.logVersion
    Group(title = stringResource(R.string.diag_title)) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp)) {
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.End,
            ) {
                TextButton(onClick = { activity.runHealthCheck() }) {
                    Text(stringResource(R.string.btn_health), fontSize = 13.sp)
                }
                TextButton(onClick = { activity.copyDiag() }) {
                    Text(stringResource(R.string.btn_copy_diag), fontSize = 13.sp)
                }
            }
            DiagHeading(stringResource(R.string.diag_net))
            DiagBody(activity.diagNetText, scheme)

            DiagHeading(stringResource(R.string.diag_audio))
            DiagBody(activity.diagAudioText, scheme)
            LevelRow(stringResource(R.string.diag_level_up), activity.upPercent,
                activity.upValueText, scheme)
            LevelRow(stringResource(R.string.diag_level_down), activity.downPercent,
                activity.downValueText, scheme)
            LevelRow(stringResource(R.string.diag_buffer), activity.bufferPercent,
                activity.bufferValueText, scheme)

            DiagHeading(stringResource(R.string.diag_state))
            DiagBody(activity.diagStatusText, scheme)
            Text(
                text = stringResource(R.string.status_health) + activity.healthText,
                modifier = Modifier.padding(top = 6.dp),
                fontSize = 10.sp,
                fontFamily = FontFamily.Monospace,
                color = scheme.onSurfaceVariant,
            )
            if (activity.lastErr.isNotEmpty()) {
                Text(
                    text = activity.lastErr,
                    modifier = Modifier.padding(top = 6.dp),
                    fontSize = 11.sp,
                    color = Color(0xFFC62828),
                )
            }

            DiagHeading(stringResource(R.string.diag_log))
            Text(
                text = activity.logSnapshot(),
                modifier = Modifier.padding(top = 4.dp, bottom = 8.dp),
                fontSize = 10.sp,
                lineHeight = 14.sp,
                fontFamily = FontFamily.Monospace,
                color = scheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun DiagHeading(text: String) {
    Text(
        text = text,
        modifier = Modifier.padding(top = 10.dp),
        fontSize = 12.sp,
        fontWeight = FontWeight.Medium,
        color = CouiTheme.colorScheme.onSurfaceVariant,
    )
}

@Composable
private fun DiagBody(text: String, scheme: CouiColorScheme) {
    Text(
        text = text,
        modifier = Modifier.padding(top = 4.dp),
        fontSize = 11.sp,
        lineHeight = 16.sp,
        fontFamily = FontFamily.Monospace,
        color = scheme.onSurface,
    )
}

/** 一条电平 / 缓冲：标签 + 进度条 + 右对齐读数（列宽固定，数值变长也不跳）。 */
@Composable
private fun LevelRow(label: String, percent: Int, value: String, scheme: CouiColorScheme) {
    Row(
        Modifier.fillMaxWidth().padding(top = 5.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = label,
            modifier = Modifier.width(72.dp),
            fontSize = 11.sp,
            color = scheme.onSurfaceVariant,
        )
        LinearProgressIndicator(
            progress = { percent.coerceIn(0, 100) / 100f },
            modifier = Modifier.weight(1f).height(6.dp),
            color = scheme.accent,
            trackColor = scheme.outlineVariant,
        )
        Text(
            text = value,
            modifier = Modifier.width(92.dp),
            fontSize = 11.sp,
            fontFamily = FontFamily.Monospace,
            textAlign = TextAlign.End,
            color = scheme.onSurface,
        )
    }
}

