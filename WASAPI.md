# WASAPI — Windows 输出兼容性（研究与实现约定）

> 结论先说：**不碰独占**，全程 `AUDCLNT_SHAREMODE_SHARED`。旧版（Qt/PortAudio）兼容性好，
> 不是因为它用了独占，而是因为它把「设备给什么格式」交给 PortAudio 转换，而不是只接受 f32。

## 1. 唯一合同：`GetMixFormat()`

Windows 共享混音格式只会是 **PCM** 或 **IEEE float**：

| 类型 | `wFormatTag` / `SubFormat` | 位深 |
| --- | --- | --- |
| PCM | `WAVE_FORMAT_PCM` / `KSDATAFORMAT_SUBTYPE_PCM` | 8 / 16 / 24 / 32 / 64 |
| float | `WAVE_FORMAT_IEEE_FLOAT` / `KSDATAFORMAT_SUBTYPE_IEEE_FLOAT` | 32 / 64 |

`WAVE_FORMAT_EXTENSIBLE` 时，真正的类型在 **`SubFormat` GUID（偏移 24）**，不是
`dwChannelMask`（偏移 20）。读错偏移的后果：立体声 mask=3 被当成 float、单声道 mask=4
被当成未知，**只有 2ch/32-bit float 碰巧能用**。

24-bit 有两种容器：

- 3 字节紧凑（`nBlockAlign = 3 × ch`）→ `Sample::I24Packed`；
- 32 位容器、有效 24 位（`wValidBitsPerSample = 24`）→ `Sample::I24In32`，数值放在**高 24 位**。

## 2. 初始化阶梯（`src/audio/wasapi_out.rs`）

按「低延迟优先、失败才退」的顺序，**周期不变、不加大缓冲**：

1. `IAudioClient3::InitializeSharedAudioStream(EVENTCALLBACK, min_period, mix)`
   —— 原生混音格式，最低延迟（VB-Cable 实测 128 帧 / 2.67 ms）；
2. `IAudioClient::Initialize(SHARED, EVENTCALLBACK, default_period, 0, mix)`
   —— 原生格式，普通共享周期；
3. `IAudioClient3` + `f32_like(mix)` + `AUTOCONVERTPCM | SRC_DEFAULT_QUALITY`
   —— 端点支持时同时拿到低延迟和引擎转换；
4. `IAudioClient::Initialize(SHARED, EVENTCALLBACK | AUTOCONVERTPCM | SRC_DEFAULT_QUALITY,
   default_period, 0, f32_like(mix))`
   —— 请求 f32，由 Windows 音频引擎转换位深/声道（PortAudio 同款兜底）。

`probe()` 仍用 `mix` 问 `GetSharedModeEnginePeriod`（低延迟能力只对混音格式成立）；
`classify()` 不认识的格式不报错，返回 `engine_convert = true`，由第 3 步兜底。

## 3. 输入侧

WASAPI **capture 不支持 `AUTOCONVERTPCM`**：必须用 `default_input_config()` 的原生格式，
由 cpal 的 `FromSample` 转 f32。覆盖：

```
U8 / I8 / I16 / U16 / I24 / U24 / I32 / U32 / I64 / U64 / F32 / F64
```

（`src/audio/capture.rs` 的 match；cpal 对 24-bit 麦克风返回 `I24`，以前直接报
“不支持的采样格式”，这是 USB 麦克风/声卡最常见的失败点。）

## 4. 重采样

引擎内部恒 48 kHz 单声道；设备率只在边界出现：

- 输入：`ToHops` 原生率 → 48 kHz（整数倍降采样走多相，非整数走 rubato `Async`）；
- 输出：`Converter` 48 kHz → 设备率（PI 伺服可微调 ±3%）。

`AUTOCONVERTPCM` 的 `SRC_DEFAULT_QUALITY` 只作为**格式兜底**，不作为主路径：主路径请求
设备混音率，不引入引擎 SRC 的额外延迟；只有混音格式不认识时才让引擎转换。

## 5. 明确不做

- **不做独占**：端点独占会让游戏/会议/浏览器打不开，且这台机器所有端点对 32-bit float
  独占都返回 `0x88890008`；
- **不靠加大缓冲换兼容**：需要更大缓冲的是端点/调度问题，不是格式问题；
- **不静默换设备**：打开失败要给明确原因（`with_device_retry` 只对 busy/disconnected 退避）。

## 6. 验证

- 单测：`classify_*`（1ch/16-bit、2ch/16-bit、24-bit、24-in-32、f32）+ `fill_packs_24_bit`；
- 运行时：`/debug` 的 `streams[]` 看 `sample_format` / `callback_frames` / `underrun_per_s`；
- 16-bit 设备：把端点默认格式改成 `16 位，48000 Hz`（用户自行操作，测试脚本不改系统设置），
  确认 `streams[]` 显示 `i16` 且 `underrun_per_s = 0`。

## 7. 参考实现

- **cpal 0.18 `src/host/wasapi/device.rs`**（本地依赖，同一份代码可查）：
  - 输出 `Initialize` 固定带 `AUTOCONVERTPCM | SRC_DEFAULT_QUALITY`；
  - capture 只用混音格式；
  - `shared_mode_period_frames()` 用 `GetDevicePeriod` 的 default 作为回调周期；
  - `format_from_waveformatex_ptr()` 用 `WAVEFORMATEXTENSIBLE_0.SubFormat` 判型。
- PortAudio `pa_win_wasapi.c`：旧版 PyAudio 的底层；同为 shared + 应用侧 float32 转换。
