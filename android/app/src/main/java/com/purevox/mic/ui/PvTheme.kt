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

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/**
 * 本地小型 COUI 风格组件（ColorOS 17 观感，但不引第三方 UI 库）。
 *
 * 第三方 COUI 把 effects / shader / 菜单 / 选择器整套打进 dex，还要求 minSdk 31；
 * 本项目只用到「圆角分组卡片 + 列表行 + 开关」三种最基础的东西。这里自己实现，
 * 外观保持一致，依赖只剩 Compose 本身，minSdk 也能回到 24。
 */
@Immutable
data class PvColorScheme(
    val accent: Color,
    val onAccent: Color,
    val surface: Color,
    val container: Color,
    val onSurface: Color,
    val onSurfaceVariant: Color,
    val outlineVariant: Color,
    val background: Color,
    val card: Color,
)

private val LightScheme = PvColorScheme(
    accent = Color(0xFF0A6CFF),
    onAccent = Color.White,
    surface = Color.White,
    container = Color.White,
    onSurface = Color(0xFF17191C),
    onSurfaceVariant = Color(0xFF6B7280),
    outlineVariant = Color(0xFFE2E7F0),
    background = Color(0xFFF2F4F8),
    card = Color.White,
)

private val DarkScheme = PvColorScheme(
    accent = Color(0xFF9BC2FF),
    onAccent = Color(0xFF0A1F3A),
    surface = Color(0xFF171A20),
    container = Color(0xFF171A20),
    onSurface = Color(0xFFECEEF2),
    onSurfaceVariant = Color(0xFF9AA4B2),
    outlineVariant = Color(0xFF2A303A),
    background = Color(0xFF0E1014),
    card = Color(0xFF171A20),
)

private val LocalPvScheme = staticCompositionLocalOf { LightScheme }

/** 当前配色（`LightScheme` / `DarkScheme`）。 */
val pvColors: PvColorScheme
    @Composable get() = LocalPvScheme.current

@Composable
fun ProvidePvTheme(darkTheme: Boolean, content: @Composable () -> Unit) {
    CompositionLocalProvider(
        LocalPvScheme provides if (darkTheme) DarkScheme else LightScheme,
        content = content,
    )
}

/** COUI 风格的 Group：标题行 + 24dp 圆角卡片 + 可选 footer。 */
@Composable
fun PvGroup(
    modifier: Modifier = Modifier,
    title: String? = null,
    footer: String? = null,
    headerActions: @Composable RowScope.() -> Unit = {},
    content: @Composable ColumnScope.() -> Unit,
) {
    val c = pvColors
    Column(modifier.fillMaxWidth().padding(top = if (title.isNullOrEmpty()) 16.dp else 12.dp)) {
        if (!title.isNullOrEmpty()) {
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 32.dp, vertical = 8.dp).heightIn(min = 16.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = title,
                    modifier = Modifier.weight(1f),
                    fontSize = 12.sp,
                    color = c.onSurfaceVariant,
                    textAlign = TextAlign.Start,
                )
                headerActions()
            }
        }
        Column(
            Modifier
                .padding(horizontal = 16.dp)
                .fillMaxWidth()
                .clip(RoundedCornerShape(24.dp))
                .background(c.card),
            content = content,
        )
        if (!footer.isNullOrEmpty()) {
            Text(
                text = footer,
                modifier = Modifier.fillMaxWidth().padding(horizontal = 32.dp, vertical = 8.dp),
                fontSize = 11.sp,
                lineHeight = 15.sp,
                color = c.onSurfaceVariant,
            )
        }
    }
}

/** COUI 风格的 Item：图标 + 标题 + 说明 + 尾部内容；整行可点。 */
@Composable
fun PvItem(
    modifier: Modifier = Modifier,
    icon: (@Composable () -> Unit)? = null,
    summary: (@Composable () -> Unit)? = null,
    trailing: @Composable () -> Unit = {},
    onClick: (() -> Unit)? = null,
    enabled: Boolean = true,
    contentPadding: PaddingValues = PaddingValues(horizontal = 16.dp, vertical = 10.dp),
    title: @Composable () -> Unit,
) {
    val click = if (onClick != null) {
        Modifier.clickable(enabled = enabled, onClick = onClick)
    } else {
        Modifier
    }
    Row(
        modifier
            .fillMaxWidth()
            .defaultMinSize(minHeight = 48.dp)
            .then(click)
            .padding(contentPadding),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) {
            Box(Modifier.padding(end = 16.dp).size(24.dp), contentAlignment = Alignment.Center) {
                icon()
            }
        }
        Column(
            Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            title()
            summary?.invoke()
        }
        trailing()
    }
}

/** COUI 风格的 Divider：卡片内的细分隔线（至少 1 物理像素）。 */
@Composable
fun PvDivider(
    modifier: Modifier = Modifier,
    startIndent: androidx.compose.ui.unit.Dp = 16.dp,
    endIndent: androidx.compose.ui.unit.Dp = 16.dp,
    color: Color = pvColors.outlineVariant,
) {
    Box(
        modifier
            .fillMaxWidth()
            .padding(start = startIndent, end = endIndent)
            .height(1.dp)
            .background(color),
    )
}

/** COUI 观感的开关：跑道/滑块配色统一走本地色板。 */
@Composable
fun PvSwitch(
    checked: Boolean,
    onCheckedChange: ((Boolean) -> Unit)?,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    val c = pvColors
    Switch(
        checked = checked,
        onCheckedChange = onCheckedChange,
        modifier = modifier,
        enabled = enabled,
        colors = SwitchDefaults.colors(
            checkedThumbColor = Color.White,
            checkedTrackColor = c.accent,
            checkedBorderColor = Color.Transparent,
            uncheckedThumbColor = Color.White,
            uncheckedTrackColor = c.outlineVariant,
            uncheckedBorderColor = Color.Transparent,
            disabledCheckedThumbColor = Color.White.copy(alpha = 0.8f),
            disabledCheckedTrackColor = c.accent.copy(alpha = 0.38f),
            disabledUncheckedThumbColor = Color.White.copy(alpha = 0.8f),
            disabledUncheckedTrackColor = c.outlineVariant.copy(alpha = 0.6f),
        ),
    )
}
