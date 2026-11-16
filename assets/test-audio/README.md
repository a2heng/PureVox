# `assets/test-audio` — 随包测试素材

「测试音」节点（`ptype = tone`）的素材来源。每份都是**单次最小瞬态**，重复节奏由节点的
`gap_ms`（重复间隙）在运行期控制；多选时相加播放。文件随二进制嵌入（`include_bytes!`），
不依赖运行目录。

| 文件 | 内容 | 源录音时间 | 峰值 |
| --- | --- | --- | --- |
| `knock.wav` | 敲桌子（一次） | 7.41–7.64 s | −2.2 dBFS |
| `mouse.wav` | 鼠标点击（一次） | 7.68–7.77 s | −22.5 dBFS |
| `keyboard.wav` | 键盘按键（一次） | 12.20–12.33 s | −15.7 dBFS |

- 格式：48 kHz / 单声道 / 16-bit PCM；两端 3 ms 淡入淡出（避免截断爆音）。
- 电平保持原始录音：鼠标 / 键盘比敲桌子轻，需要的电平由测试音节点的「输入微调」补。
- 素材来自用户提供的真实录音，只随 PureVox 用于自测；不在其他项目复用。

## 重新生成

源录音不入库。把源文件放任意位置，然后：

```bash
PUREVOX_CLIP_SOURCE="C:/path/to/record.mp3" \
  cargo test --bin purevox -- --ignored --nocapture make_test_clips
```

生成逻辑在 `src/infer/denoise.rs` 的 `make_test_clips`：解码走 `media::decode_to_mono_48k`
（与媒体库导入同一份），写 WAV 走 `wav::write_mono_16`（与 TSE 参考同一份）。
改截取区间时同时改本表。
