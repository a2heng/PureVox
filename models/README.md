# `models/` — 内置模型（不进 GPL，见 `MODEL-LICENSE.md`）

**文件名固定、不带版本号**（2026-10-29 起）。应用代码只认这些名字
（`infer::MODEL_DENOISE` / `infer::MODEL_DENOISE_M` / `infer::MODEL_DENOISE_L` /
`infer::MODEL_DENOISE_5A` / `infer::MODEL_DENOISE_5B` /
`infer::aec::MODEL_AEC` / `infer::tse::MODEL_TSE` / `infer::tse::MODEL_TSE_REF`）⇒
**换模型 = 换文件**，一行代码都不用改。
降噪有**五个档**（界面下拉「降噪（小）/（中）/（大）/（5A）/（5B）」），
2026-11-18 起**全部只是换模型**：没有深度旋钮、没有运行期参数差异。

| 文件 | 版本（代数 @ epoch） | 接口（每 hop 10 ms / 48 kHz） | cache |
|---|---|---|---|
| `purevox_denoise.onnx`（小） | `purevox_denoise_202611d` @ **E110** | `spec_in (1,2,481) + cache_in + ctrl_in (1,2)` → `spec_out + cache_out` | 25,140 |
| `purevox_denoise_medium.onnx`（中） | `purevox_denoise_202611e` @ **E76** | 同上（带 `ctrl_in`，应用侧固定喂 -240） | 22,964 |
| `purevox_denoise_large.onnx`（大） | `purevox_denoise_202611f` @ **E128** | 同上（带 `ctrl_in`，应用侧固定喂 -240） | 31,860 |
| `purevox_denoise_5a.onnx`（5A） | `purevox_denoise_202605a` @ **E389** | `spec_in (1,2,481) + cache_in` → `spec_out + cache_out`（**无 `ctrl_in`**） | 10,208 |
| `purevox_denoise_5b.onnx`（5B） | `purevox_denoise_202605b` @ **E227** | 同上（**无 `ctrl_in`**） | 14,024 |
| `purevox_aec.onnx` | `purevox_aec_202610` @ **E381** | `mic_spec + far_spec + cache_in` → `mic_spec_out + cache_out` | 91,920 |
| `purevox_tse.onnx` | `purevox_tse_202609c` @ **E205** | `mix_spec (1,2,481) + enr_tok (1,2,1001,180) + cache_in` → `spec_out + cache_out` | 513,216 |
| `purevox_tse_ref_encoder.onnx` | `purevox_tse_202609c` @ **E205**（与主图同批） | `ref_spec (1,2,1001,481)` → `enr_tok (1,2,1001,180)`（每句一次，参考 10 s） | — |

**版本号怎么读**：`model` 是 ONNX metadata 里的代数（`onnx.load('…').metadata_props`），
epoch 记在这张表里。换模型时先改这张表，再改文件（表是部署现状的唯一记录）。

- **`ctrl_in` 已不再是界面参数（2026-11-18 起）**：11 系（小/中/大）图里仍有
  `ctrl_in (1,2) = [floor_db, depth_gain]`，但应用侧**固定喂 `[-240, 1.0]`**——
  -240 就是训练期的 `MASK_FLOOR_OFF`（不夹 = 裸 sigmoid = 最深），所以等于「拉满」。
  202605a/b（5A/5B）**没有**这个输入，训练时本身就是这个行为。
  ⇒ 界面上没有「降噪深度」滑块了，五档降噪只靠 `model` 切换；旧计划里的 `depth_db` 在
  读取时直接清掉（`plan::normalize`）。
- **`cache_in` 维度随模型变**：应用侧运行时用 `cache_dim()` 从 session 输入形状读，不写死；
  但**模型池按路径缓存 session**，所以**替换文件后必须重启应用**才生效。
- **谱接口**：图内只有网络，输入/输出都是 `(1,2,481)` 复数谱；960/480 周期 Hann 分析与合成、
  OLA（逐样本除 Σw²）、1 hop 滞后都在应用侧（`src/infer/spec.rs`，三类模型共用一份）。
- **TSE 的两个文件必须成对**：`ref_encoder` 里是参考通路的权重，跟主图不配套时形状对不上。
  应用侧的契约**不写死名字**：取「主图输入里除 `spec_in`/`mix_spec` / `cache_in` 之外的项」当参考条件
  （202609c 线是 `enr_tok`，202610 线是 `keys`+`gb`，202611 线是 `enr_comp`），
  注册编码器必须**同名输出、形状逐个对上**，另比 metadata `model`；不配套直接报错（硬失败），
  不会退化成直通。
- **降噪五档现状**：11 系三档是 2026-11-16 换代（同一套谱接口 + `ctrl_in`），
  5A/5B 是 2026-11-17 回退的 2026-05 ulunas M/L（`spec_in + cache_in`，无 `ctrl_in`）。
  界面下拉「小 / 中 / 大 / 5A / 5B」，默认小档。
  - **小档 `202611d@E110`**：cache **25,140**，文件 **0.88 MiB**，
    单线程单 hop（含图外 FFT）**0.375 ms / RTF 0.038**。
  - **中档 `202611e@E76`**：cache **22,964**，文件 **0.99 MiB**，**0.429 ms / RTF 0.043**。
  - **大档 `202611f@E128`**：cache **31,860**，文件 **1.08 MiB**，**0.469 ms / RTF 0.047**。
  - **5A `202605a@E389`**：cache **10,208**，文件 **2.49 MB**，**0.403 ms·hop⁻¹ / RTF 0.040**。
  - **5B `202605b@E227`**：cache **14,024**，文件 **2.43 MB**，**0.460 ms·hop⁻¹ / RTF 0.046**。
  - 5A/5B 的 ONNX 由训练侧
    `8_purevox_mic_net/publish_to_app.py --denoise-5a 389 --denoise-5b 227` 发布
    （会内联外部数据并跑一次 ORT 前向校验）。

- **TSE 是 `202609c@E205`（2026-11-04 更新）**：从训练侧
  `7_output/output_purevox_tse_202609c/checkpoints/checkpoint_epoch_205.tar` 用
  `legacy/202609/purevox_tse_202609c.py` 的 `export_onnx` / `export_ref_encoder` 导出
  （`PureVoxTse202609CStreamSpec`：**傅里叶在图外**，图内只有网络）。
  480 hop / 960 窗与引擎 10 ms 网格一致；参考 1001 帧 = 10 s；`enr_tok` 每 hop 原样重喂。
  实测（`6_epoch_end_test_wav/tse`，mix→目标 −4.15 dB）：输出→目标 **+5.34 dB（Δ +9.5 dB）**。
- ⚠️ **逐位全零**输入可能出 NaN（2026-10-31 实测 202611a：图内归一 0/0；202609c / 202610 线没这
  毛病，但保不齐下一代又踩）。应用侧对这一 hop 补零 + 清缓存并记 warn（`infer/tse.rs`）；
  根治在训练侧（归一分母加 eps）。所以拿全零谱做前向自检会被误判成坏模型 —— 喂非零谱即可。
- **替换流程**：TSE 成对文件必须同批替换；降噪五档各自独立（换哪个档只动哪个文件）。替换后重启应用。
- 每个文件里的 ONNX metadata 记着模型标识（`model=…`），可用
  `python -c "import onnx;print(onnx.load('…').metadata_props)"` 查看。

**为什么文件名不带版本号**：版本号进文件名就必然要在代码里同步改名，"换模型"就又变成
"改代码 + 重新发版"。现在的约定是文件内的 metadata 记标识、本表记部署现状，替换只动文件。
`5a` / `5b` 是**并存的模型身份**（2026-05 ulunas M/L），不是 epoch 版本号；换这两档的
epoch 同样只覆盖同名文件。
