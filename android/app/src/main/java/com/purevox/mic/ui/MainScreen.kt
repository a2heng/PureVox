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

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.drag
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
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
import androidx.compose.foundation.progressSemantics
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ColorScheme
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
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.setProgress
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import com.purevox.mic.MainActivity
import com.purevox.mic.R
import com.purevox.mic.audio.MicSources
import com.purevox.mic.update.UpdateChecker
import kotlin.math.roundToInt

/** 右侧 hero 上方的品牌色：与旧 XML 的头图渐变一致（深蓝 → 亮一点的蓝）。 */
private val HeroTop = Color(0xFF132C50)
private val HeroBottom = Color(0xFF2C5C9F)
/** hero 上的次级文字（副标题 / 目标地址）。 */
private val HeroMuted = Color(0xFFB9D2F5)
/** 就绪状态的大字（偏绿的亮色，与深蓝底对比够）。 */
private val HeroReady = Color(0xFFC7F7DA)

/**
 * 界面根：本地 PvTheme + Material 配色桥接。
 *
 * 为什么还要包一层 [MaterialTheme]：输入框 / 按钮直接用 Material 3；
 * 把本地色板映射进 Material 的 `colorScheme`，才不会出现 M3 默认的紫色。
 */
@Composable
internal fun PureVoxApp(activity: MainActivity) {
    // 订阅 2 Hz 的状态刷新：updateStatus() 末尾会 uiTick++，Compose 根跟着重算一遍
    activity.uiTick

    ProvidePvTheme(darkTheme = isSystemInDarkTheme()) {
        val scheme = pvColors
        MaterialTheme(colorScheme = scheme.toMaterialColorScheme()) {
            Box(Modifier.fillMaxSize().background(scheme.background)) {
                HomeScreen(activity)
                if (activity.settingsOpen) SettingsDialog(activity)
            }
        }
    }
}

/** 本地色板 → Material 3 色板；只映射实际用到的角色。 */
@Composable
private fun PvColorScheme.toMaterialColorScheme(): ColorScheme {
    val dark = isSystemInDarkTheme()
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
    val scheme = pvColors
    Column(
        Modifier
            .fillMaxSize()
            .imePadding()
            .verticalScroll(rememberScrollState()),
    ) {
        Hero(activity)

        // 三个开关 = 三个「现在要不要发生」，整行可点，不是只有开关本体
        PvGroup {
            SwitchRow(
                text = stringResource(R.string.sw_connect),
                checked = activity.connectOn,
                enabled = !activity.connecting,
                onCheckedChange = { activity.setConnect(it) },
            )
            PvDivider()
            SwitchRow(
                text = stringResource(R.string.sw_mic),
                checked = activity.micOn,
                enabled = activity.ready,
                onCheckedChange = { activity.setMic(it) },
            )
            PvDivider()
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
            modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 10.dp),
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
                .padding(start = 20.dp, top = 8.dp, end = 14.dp, bottom = 12.dp),
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
                        painter = painterResource(R.drawable.ic_settings),
                        contentDescription = stringResource(R.string.btn_settings),
                        tint = Color.White,
                        modifier = Modifier.size(21.dp),
                    )
                }
            }

            Row(
                modifier = Modifier.padding(top = 8.dp),
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
        }
    }
}

/**
 * 统一的主操作 / 次操作按钮：**胶囊形、无阴影、48dp 高**。
 *
 * 全应用只从这里出按钮（连接目标 / 保活 / 更新 / 诊断），不再混用 M3 默认外观，
 * 也不让某处退化成「很细的文字」。
 */
@Composable
private fun PvButton(
    text: String,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    icon: Int? = null,
    height: Dp = 48.dp,
    onClick: () -> Unit,
) {
    val scheme = pvColors
    Button(
        onClick = onClick,
        modifier = modifier.height(height),
        enabled = enabled,
        shape = RoundedCornerShape(16.dp),
        colors = ButtonDefaults.buttonColors(
            containerColor = scheme.accent,
            contentColor = scheme.onAccent,
            disabledContainerColor = scheme.accent.copy(alpha = 0.12f),
            disabledContentColor = scheme.onSurface.copy(alpha = 0.38f),
        ),
        elevation = ButtonDefaults.buttonElevation(
            defaultElevation = 0.dp,
            pressedElevation = 0.dp,
        ),
        contentPadding = PaddingValues(horizontal = 16.dp),
    ) {
        if (icon != null) {
            Icon(painterResource(icon), null, modifier = Modifier.size(18.dp))
            Spacer(Modifier.width(6.dp))
        }
        Text(text = text, fontSize = 14.sp, maxLines = 1)
    }
}

/** 统一的次操作按钮：可见描边，和 [PvButton] 同高同圆角。 */
@Composable
private fun PvOutlinedButton(
    text: String,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    icon: Int? = null,
    height: Dp = 48.dp,
    onClick: () -> Unit,
) {
    val scheme = pvColors
    OutlinedButton(
        onClick = onClick,
        modifier = modifier.height(height),
        enabled = enabled,
        shape = RoundedCornerShape(16.dp),
        border = BorderStroke(
            1.dp,
            if (enabled) scheme.outlineVariant else scheme.outlineVariant.copy(alpha = 0.5f),
        ),
        colors = ButtonDefaults.outlinedButtonColors(
            contentColor = scheme.onSurface,
            disabledContentColor = scheme.onSurface.copy(alpha = 0.38f),
        ),
        contentPadding = PaddingValues(horizontal = 16.dp),
    ) {
        if (icon != null) {
            Icon(painterResource(icon), null, modifier = Modifier.size(18.dp))
            Spacer(Modifier.width(6.dp))
        }
        Text(text = text, fontSize = 14.sp, maxLines = 1)
    }
}

/**
 * 普通滑块：**外形像 COUI 胶囊滑条，但不带任何特效**。
 *
 * COUI 的 Slider 自带 spotlight / 形变 / 跟手动效，在当前 ROM 上有绘制 bug；
 * 这里用 Canvas 画一条 16dp 胶囊轨道 + 白色圆环 thumb，自己处理拖动/点击，
 * 不接任何 shader、光斑或动画。步长 / 语义仍完整（无障碍可调）。
 */
@Composable
private fun PvSlider(
    value: Float,
    valueRange: ClosedFloatingPointRange<Float>,
    steps: Int,
    onValueChange: (Float) -> Unit,
    modifier: Modifier = Modifier,
) {
    val scheme = pvColors
    val span = valueRange.endInclusive - valueRange.start
    val stepSize = if (steps > 0) span / (steps + 1) else 0f
    fun snap(v: Float): Float {
        val clamped = v.coerceIn(valueRange.start, valueRange.endInclusive)
        return if (stepSize > 0f) {
            (valueRange.start + ((clamped - valueRange.start) / stepSize).roundToInt() * stepSize)
                .coerceIn(valueRange.start, valueRange.endInclusive)
        } else {
            clamped
        }
    }

    val onValue by rememberUpdatedState(onValueChange)
    Canvas(
        modifier
            .fillMaxWidth()
            .height(48.dp)
            .progressSemantics(value, valueRange, steps)
            .semantics {
                setProgress { target ->
                    onValue(snap(target))
                    true
                }
            }
            .pointerInput(valueRange, steps) {
                val pad = 12.dp.toPx()
                val usable = (size.width - pad * 2).coerceAtLeast(1f)
                fun update(x: Float) {
                    val f = ((x - pad) / usable).coerceIn(0f, 1f)
                    onValue(snap(valueRange.start + f * span))
                }
                awaitEachGesture {
                    val down = awaitFirstDown()
                    update(down.position.x)
                    drag(down.id) { change ->
                        update(change.position.x)
                        change.consume()
                    }
                }
            },
    ) {
        val pad = 12.dp.toPx()
        val usable = (size.width - pad * 2).coerceAtLeast(1f)
        val fraction = ((value - valueRange.start) / span).coerceIn(0f, 1f)
        val thumbX = pad + fraction * usable
        val cy = size.height / 2f
        val trackH = 16.dp.toPx()
        val radius = trackH / 2f

        drawRoundRect(
            color = scheme.outlineVariant,
            topLeft = Offset(pad, cy - radius),
            size = Size(usable, trackH),
            cornerRadius = CornerRadius(radius, radius),
        )
        drawRoundRect(
            color = scheme.accent,
            topLeft = Offset(pad, cy - radius),
            size = Size((thumbX - pad).coerceAtLeast(0f), trackH),
            cornerRadius = CornerRadius(radius, radius),
        )
        val thumbR = 11.dp.toPx()
        drawCircle(color = Color.White, radius = thumbR, center = Offset(thumbX, cy))
        drawCircle(
            color = scheme.accent,
            radius = thumbR,
            center = Offset(thumbX, cy),
            style = Stroke(width = 2.dp.toPx()),
        )
    }
}

/** 一行开关：整行可点（本地 PvItem 提供行内留白；开关本身不再接第二份点击）。 */
@Composable
private fun SwitchRow(
    text: String,
    checked: Boolean,
    enabled: Boolean,
    onCheckedChange: (Boolean) -> Unit,
) {
    val scheme = pvColors
    PvItem(
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
            PvSwitch(
                checked = checked,
                onCheckedChange = null,
                enabled = enabled,
            )
        },
    )
}

/** 连接目标：两个输入框 + 扫码 + 两个全宽操作按钮。 */
@Composable
private fun TargetSection(activity: MainActivity, scheme: PvColorScheme) {
    val fieldColors = OutlinedTextFieldDefaults.colors(
        focusedBorderColor = scheme.accent,
        unfocusedBorderColor = scheme.outlineVariant,
        focusedTextColor = scheme.onSurface,
        unfocusedTextColor = scheme.onSurface,
        cursorColor = scheme.accent,
        focusedPlaceholderColor = scheme.onSurfaceVariant,
        unfocusedPlaceholderColor = scheme.onSurfaceVariant,
        focusedContainerColor = scheme.background,
        unfocusedContainerColor = scheme.background,
    )
    PvGroup(title = stringResource(R.string.section_target)) {
        Column(Modifier.fillMaxWidth().padding(12.dp)) {
            Row(
                Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                // IP 占**正好一半**：右边缘与下面两个按钮的中缝对齐
                OutlinedTextField(
                    value = activity.hostInput,
                    onValueChange = { activity.hostInput = it },
                    modifier = Modifier.weight(1f),
                    singleLine = true,
                    shape = RoundedCornerShape(14.dp),
                    placeholder = { Text(stringResource(R.string.hint_host), fontSize = 13.sp) },
                    textStyle = MaterialTheme.typography.bodyMedium.copy(fontSize = 15.sp),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                    colors = fieldColors,
                )
                // 另外一半：端口 + 扫码（扫码与端口同一半，保持和按钮同一条中线）
                Row(
                    Modifier.weight(1f),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    OutlinedTextField(
                        value = activity.portInput,
                        onValueChange = { raw ->
                            // 端口只收数字、最多 5 位；非法值在 readTarget() 里回退到默认端口
                            activity.portInput = raw.filter { it in '0'..'9' }.take(5)
                        },
                        modifier = Modifier.weight(1f),
                        singleLine = true,
                        shape = RoundedCornerShape(14.dp),
                        placeholder = { Text(stringResource(R.string.hint_port), fontSize = 13.sp) },
                        textStyle = MaterialTheme.typography.bodyMedium.copy(fontSize = 15.sp),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                        colors = fieldColors,
                    )
                    // 扫码不是「一个不起眼的图标」：52dp 圆形按钮 + 强调色
                    Box(
                        Modifier
                            .size(52.dp)
                            .clip(CircleShape)
                            .background(scheme.accent.copy(alpha = 0.12f))
                            .clickable { activity.startScan() },
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(
                            painter = painterResource(R.drawable.ic_scan),
                            contentDescription = stringResource(R.string.btn_scan),
                            tint = scheme.accent,
                            modifier = Modifier.size(26.dp),
                        )
                    }
                }
            }
            Row(
                Modifier.fillMaxWidth().padding(top = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                PvOutlinedButton(
                    text = stringResource(R.string.btn_rediscover),
                    modifier = Modifier.weight(1f),
                    icon = R.drawable.ic_notif_search,
                    onClick = { activity.rediscover() },
                )
                PvButton(
                    text = stringResource(R.string.btn_reconnect),
                    modifier = Modifier.weight(1f),
                    icon = R.drawable.ic_notif_refresh,
                    onClick = { activity.reconnect() },
                )
            }
        }
    }
}

/**
 * 抖动缓冲 + 麦克风 API：**合成一张卡**。
 *
 * 麦克风 API 只是一个标题行右侧的小按钮（`API · 默认`），不单独占一张卡 / 一行：
 * 它属于「采集参数」，不是另一个功能块；点开仍是本地选择器。
 */
@Composable
private fun AudioSection(activity: MainActivity, scheme: PvColorScheme) {
    var pickerOpen by remember { mutableStateOf(false) }
    PvGroup(
        title = stringResource(R.string.buffer_title_value, activity.ringMs / 10, activity.ringMs),
        headerActions = {
            Row(
                Modifier
                    .clip(RoundedCornerShape(50))
                    .background(scheme.accent.copy(alpha = 0.12f))
                    .clickable { pickerOpen = true }
                    .padding(horizontal = 10.dp, vertical = 5.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = "API · " + MicSources.shortLabel(activity, activity.micSourceId),
                    modifier = Modifier.widthIn(max = 100.dp),
                    fontSize = 10.sp,
                    color = scheme.accent,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Icon(
                    painter = painterResource(R.drawable.ic_chevron_right),
                    contentDescription = stringResource(R.string.mic_source_label),
                    tint = scheme.accent,
                    modifier = Modifier.padding(start = 2.dp).size(14.dp),
                )
            }
        },
    ) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp)) {
            PvSlider(
                value = activity.ringMs.toFloat(),
                onValueChange = { activity.setRingTarget(it.toInt()) },
                valueRange = 10f..200f,
                steps = 18,
            )
        }
    }
    if (pickerOpen) MicSourcePicker(activity, scheme) { pickerOpen = false }
}

/** 麦克风 API 选择器：本地圆角卡片 + 选中勾。 */
@Composable
private fun MicSourcePicker(activity: MainActivity, scheme: PvColorScheme, onDismiss: () -> Unit) {
    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Box(
            Modifier
                .fillMaxSize()
                .windowInsetsPadding(WindowInsets.safeDrawing)
                .padding(horizontal = 24.dp, vertical = 40.dp),
            contentAlignment = Alignment.Center,
        ) {
            Column(
                Modifier
                    .fillMaxWidth()
                    .heightIn(max = 520.dp)
                    .clip(RoundedCornerShape(24.dp))
                    .background(scheme.container),
            ) {
                Row(
                    Modifier.fillMaxWidth().padding(start = 20.dp, top = 12.dp, end = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = stringResource(R.string.mic_source_label),
                        modifier = Modifier.weight(1f),
                        fontSize = 20.sp,
                        fontWeight = FontWeight.Bold,
                        color = scheme.onSurface,
                    )
                    IconButton(onClick = onDismiss) {
                        Icon(
                            painter = painterResource(R.drawable.ic_close),
                            contentDescription = stringResource(R.string.sheet_close),
                            tint = scheme.onSurfaceVariant,
                        )
                    }
                }
                Text(
                    text = stringResource(R.string.mic_source_hint),
                    modifier = Modifier.padding(horizontal = 20.dp, vertical = 10.dp),
                    fontSize = 11.sp,
                    lineHeight = 15.sp,
                    color = scheme.onSurfaceVariant,
                )
                HorizontalDivider(color = scheme.outlineVariant)
                Column(
                    Modifier
                        .weight(1f, fill = false)
                        .verticalScroll(rememberScrollState()),
                ) {
                    MicSources.ALL.forEachIndexed { index, source ->
                        val selected = source.id == activity.micSourceId
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .clickable {
                                    onDismiss()
                                    activity.selectMicSource(source.id)
                                }
                                .padding(horizontal = 20.dp, vertical = 14.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                text = stringResource(source.labelRes),
                                modifier = Modifier.weight(1f),
                                fontSize = 15.sp,
                                fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Normal,
                                color = if (selected) scheme.accent else scheme.onSurface,
                            )
                            if (selected) {
                                Icon(
                                    painter = painterResource(R.drawable.ic_check),
                                    contentDescription = null,
                                    tint = scheme.accent,
                                    modifier = Modifier.size(20.dp),
                                )
                            }
                        }
                        if (index != MicSources.ALL.lastIndex) {
                            PvDivider(startIndent = 20.dp, endIndent = 20.dp, color = scheme.outlineVariant)
                        }
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ 设置

@Composable
private fun SettingsDialog(activity: MainActivity) {
    val scheme = pvColors
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
                            painter = painterResource(R.drawable.ic_close),
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
                        .padding(bottom = 12.dp),
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
    val scheme = pvColors
    val current = activity.currentLanguageTag()
    val options = listOf(
        "" to stringResource(R.string.lang_system),
        "zh" to stringResource(R.string.lang_zh),
        "en" to stringResource(R.string.lang_en),
    )
    PvGroup(title = stringResource(R.string.section_language)) {
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
private fun KeepaliveSection(activity: MainActivity, scheme: PvColorScheme) {
    PvGroup(title = stringResource(R.string.section_keepalive)) {
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
                PvOutlinedButton(
                    text = stringResource(R.string.btn_notif_settings),
                    modifier = Modifier.weight(1f),
                    onClick = { activity.openNotifSettings() },
                )
                PvOutlinedButton(
                    text = stringResource(R.string.btn_battery_settings),
                    modifier = Modifier.weight(1f),
                    onClick = { activity.requestIgnoreBattery() },
                )
            }
        }
    }
}

@Composable
private fun UpdateSection(activity: MainActivity, scheme: PvColorScheme) {
    PvGroup(title = stringResource(R.string.section_update)) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp)) {
            Text(
                text = activity.updateText,
                fontSize = 13.sp,
                lineHeight = 18.sp,
                color = scheme.onSurface,
            )
            PvOutlinedButton(
                text = stringResource(R.string.btn_check_update),
                modifier = Modifier.fillMaxWidth().padding(top = 10.dp),
                enabled = !activity.updateBusy,
                onClick = { activity.checkUpdate() },
            )
            Row(
                Modifier.fillMaxWidth().padding(top = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                PvOutlinedButton(
                    text = stringResource(R.string.btn_download),
                    modifier = Modifier.weight(1f),
                    enabled = !activity.updateBusy && activity.updateRelease != null &&
                        activity.updateApk == null,
                    onClick = { activity.downloadUpdate() },
                )
                PvButton(
                    text = stringResource(R.string.btn_install),
                    modifier = Modifier.weight(1f),
                    enabled = !activity.updateBusy && activity.updateApk != null,
                    onClick = { activity.installUpdate() },
                )
            }
        }
    }
}

@Composable
private fun AboutSection(activity: MainActivity, scheme: PvColorScheme) {
    PvGroup(title = stringResource(R.string.section_about)) {
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
private fun DiagSection(activity: MainActivity, scheme: PvColorScheme) {
    // 日志是普通集合：读一次追加计数，日志文本才会跟着新行重组
    @Suppress("UNUSED_VARIABLE") val logTick = activity.logVersion
    PvGroup(title = stringResource(R.string.diag_title)) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp)) {
            Row(
                Modifier.fillMaxWidth().padding(top = 4.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                PvOutlinedButton(
                    text = stringResource(R.string.btn_health),
                    modifier = Modifier.weight(1f),
                    height = 40.dp,
                    onClick = { activity.runHealthCheck() },
                )
                PvOutlinedButton(
                    text = stringResource(R.string.btn_copy_diag),
                    modifier = Modifier.weight(1f),
                    height = 40.dp,
                    onClick = { activity.copyDiag() },
                )
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
        color = pvColors.onSurfaceVariant,
    )
}

@Composable
private fun DiagBody(text: String, scheme: PvColorScheme) {
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
private fun LevelRow(label: String, percent: Int, value: String, scheme: PvColorScheme) {
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

