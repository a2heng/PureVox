# 更新日志

## 2026-09-10 — 工具条按钮图标改用 FontAwesome 独立渲染

- 「退出 / 添加 / 设置」三个工具条按钮的图标（电源 / 加号 / 齿轮）从正文
  文本中的码点改为随包 FontAwesome 字体单独渲染的图标 Label：此前图标
  码点混在正文字体里，能否显示取决于系统字体回退，缺字形时会显示为
  方框或不显示；现图标必定呈现，无 FontAwesome 时自动退化为纯文字按钮；
- FlatButton 组件支持可选 icon 参数（图标与中文正文拆分为两个 Label——
  Tk 单 Label 无法混排两个字体族），运行态文字/底色/状态切换行为不变。

## 2026-09-05 — AEC far/mic 改外部时钟时间戳配对（GridHistory）+ 移除 FarSync/FarTap

- **far 与 mic 按外部时钟（QPC/perf 秒）配对**：mic 每 hop 带采集时间戳
  （PortAudio `input_buffer_adc_time` 经 `LinearClock` 映射到 QPC），far 样本
  带时间戳（WASAPI loopback 的 QPC）入 48k 网格历史 `GridHistory`；模型 far
  输入直接取「mic 时刻 − far_delay」的历史段（回声源），谁先到/晚到只影响
  有没有样本、不影响对齐——时间原点严格一致，无任何“为吸时钟差保持水位/
  内容滞后”的隐藏缓冲（旧 FarSync/FarTap 的 ~80ms 隐藏滞后是运行时不如
  epoch-end 的主因）。Linux 后端先以采集/读取边界的同一主时钟近似，后续接
  libpulse 流时间精化；
- **far_delay 在网格域直接切片**：far 参考 = far 历史 [mic网格 − d, +hop)，
  无需延迟环，方向/取值唯一；参考音量只缩放 far；cache 逐 hop 续传、行私有；
- **回环输入行（桌面输入）同一套时间戳网格**：回环样本带 ts 入 `GridHistory`，
  逐 hop 出队进混音；
- **far 历史不足（刚启动/缺配/生产空洞）→ 该行直通 mic**：不丢人声、不进
  模型、不动 cache；
- **删除** `pvengine/dsp/far_sync.py`（FarSync/FarTap）及其中间替代 HopQueue/
  HopFeeder 计数配队、`tests/test_far_sync.py`、临时脚本
  `test_aec_offline.py`/`test_aec_integration.py`；`pvengine` 导出改
  `GridHistory`/`grid_from_ts`；`tests/test_aec_rows.py` 重写为时间戳配对语义
  （far 历史切片、不足直通、手动延迟方向），新增 `GridHistory` 合成测试。
  捕获层新增公共件 `LinearClock`（PortAudio→QPC 映射）与 `TimedFifo`（带 ts
  FIFO），Windows far=loopback QPC / mic adc 映射均经此统一到外部钟。

## 2026-09-05 — AEC far 延迟校准重做（离线测一次）+ far=扬声器按所选端点回环

- **自动校准改回离线、可控、一次保存（唯一值）**：须先关闭音频处理、环境
  安静，播放探测音后测一次并一直使用。返回 = 「扬声器端点 → 麦克风回声」
  的**唯一延迟路径值**，即滑杆要用的数，**无任何隐含/叠加延迟**（不做
  「扣 FarTap 内置量」之类的二次处理；远/近端残余错位由模型自带多抽头
  对齐消化）。硬件不变则测得值基本恒定（实测本机稳定 ~130ms 量级，逐次
  ±5ms）。旧算法测「开始录音 → 回声进麦」绝对时长混入预卷/前导静音/输出
  缓冲等软件延迟（数百 ms 且跳动）；现改为同一时点采集 far 参考与麦克风
  后直接互相关，共同链路相消。探测音用单发上扫 chirp（无重复假峰）。校准
  失败保留原 far 延迟并提示，不再清 0；
- **far=扬声器回环改为按所选设备开采集（Windows）**：此前 WASAPI loopback
  恒抓「系统默认渲染设备」，行里选的非默认扬声器根本不进回环——回声参考
  与真实回声源不一致，延迟也对不上。现在端点选择与主设备选择一致：按名字
  相似度模糊匹配活动渲染端点（匹配不到才回退默认）。顺带修复立体声端点
  按单声道误读、以及设备切换后采集线程不跟随新客户端的问题。

## 2026-09-05 — 内置随包字体注册修复，界面正文字体与关闭钮图标生效

- **随包字体注册修复**：assets/fonts 内置字体（文泉驿微米黑中文字体、
  FontAwesome 图标字体）此前从未注册成功——字体目录被错误解析到不存在
  的 uitk/assets/fonts，注册函数静默空转，界面字体一直回退系统默认。
  现按 PyInstaller 资源目录 → 仓库根多级探测定位字体目录，Windows 经
  AddFontResourceExW（仅本进程可见，不污染系统字体表）注册目录内全部
  字体并广播 WM_FONTCHANGE 刷新 Tk 字体族缓存；Linux/macOS 仍走
  fontconfig 用户字体目录。
- **界面正文字体**：优先使用随包中文字体文泉驿微米黑（同一字体在中文
  Windows 按「文泉驿微米黑」枚举、英文环境按「WenQuanYi Micro Hei」
  枚举，两种名字均已适配），其后依次回退 YaHei Consolas Hybrid /
  Ark Pixel 像素字体系 / 微软雅黑 / Noto Sans CJK / Segoe UI；图标
  字体不再混入正文候选。
- **标题栏关闭钮**：FontAwesome 注册成功时显示窗口关闭图标，注册失败
  回退普通字符 ×，不再出现图标位空白（原实现把 Pillow 读到的族名元组
  直接传给 Tk 字体构造，族名非法被静默替换，图标码点丢失）。

## 2026-09-03 — 运行中勾选 TSE 即时生效（免重启）+ viz 未勾选空行修复

- **修复运行中启用 TSE 无效果**：TSE 参考音频此前只在启动时加载一次，
  运行中勾选 TSE 行只翻开关、插件无参考直接直通，听起来像"没启动"，
  只能停了重开。现在运行中启用即时载入已存参考并提示；全插件审计结论：
  EQ/滑杆/音效板/音乐/桌面声音均为真直播生效，输入·输出·回声消除·回环·
  远程行改 endpoint 本来就走重启，无同类问题；
- **修复 viz 行未勾选启动只剩标题**：VU/频谱行内嵌控件此前只在勾选时挂载，
  未勾选启动就是空行。现在无条件挂载，未勾选时只显示不喂数，勾选即恢复。

## 2026-09-03 — AEC 改为输入行 + 新增桌面输入：far 二选一、mic 增益只作用本路

- **回声消除从 fx 链搬到输入行（重要）**：`echo_cancel` 改为 input 种节点，
  一行一路 AEC（mic 设备 + 行内麦克风 dB + far 参考源），旧 fx 节点与全局
  far 采集整体删除。mic 与音频输入行继承同一套设备机制（同解析、同列表、
  同 48k 门禁），同设备普通输入行被接管时跳过并告警；AEC 行本身计入输入，
  只有 AEC 行（无普通输入/无媒体/无推流）也可建流，不再报无输入；
- **far 参考源二选一**：扬声器（回环采集）或另一路麦克风（专用采集，不进
  混音），行内 far 下拉分组二选一。far 经行内 `FarTap` 按 mic 主时钟拉齐后
  直达行内 `AecRow`，不经过任何处理；行头「参考 dB」滑杆只缩放进模型的
  far 帧（默认 0dB，拖动实时生效，用于调试参考电平对效果的影响）；
- **新增桌面输入行**（代码名 `loopback`）：一路扬声器播出直采为输入，拉齐后
  进混音；与 AEC 行 far=扬声器继承同一套回环采集机制。旧 `AecStage`、
  `EchoCancelPlugin`、`process_with_far`、全局 far 开关与运行时 far 切换已删除，
  行配置变更走重启（与输入行一致）；
- **约束收敛**：Windows 单输入后端要求 AEC 行 mic 与主输入同设备，网络输入
  模式不支持 AEC 行（均为阻断提示）；`aec_far_sink_*` 配置键保留占位，
  运行时只认行配置。新增合成测试 `tests/test_aec_rows.py`（FarTap/AecRow 桩
  会话）与会话计划用例（接管/路由/计数），均已登记进 `tests/run_all.py`。

## 2026-09-03 — AEC far 端时钟对齐：新增 FarSync，参考信号永不断档

- **采集侧时钟域收敛到唯一组件**：新增 `pvengine/dsp/far_sync.py`
  `FarSync`——与 `PlaybackSink` 对偶的采集方向时钟域翻译器。mic hop
  是主时钟：far 设备侧按 far 时钟 push，处理线程每 mic hop 按 mic 节奏
  pull 恰好一帧；两侧速率差（实测 ±2%）与调度抖动由 PI 伺服变速
  （±3% ASRC）消化，水位恒定。旧逻辑每 hop 刚性 `read(far_need)`，
  far 稍慢即整帧填零（参考断档致 AEC 失配抽动）、far 稍快即环满静默
  丢最旧（参考跳变），现两类断档均消除：pull 恒满帧，欠载时保持上一值
  向零衰减过渡并计数（不再填零帧），过载封顶丢最旧防延迟爬升；
- **处理循环改走对齐器**：`audio_processor` 每 hop 先把 far 环内全部可用
  样本搬入 `FarSync` 再 pull，删除缺数填零分支与启动喂零预热（预热由
  对齐器 prime 水位承担）；AEC 诊断行同步输出对齐器水位/速率/conceal/drop；
- **新增合成测试 `tests/test_far_sync.py`**（已登记进 `tests/run_all.py`）：
  标称连续、±2% 长跑无填零帧且伺服收敛到真实速率差、到达抖动连续、
  断流 conceal 平滑恢复、突发封顶有界（对标 `test_playback_sink` 的突发口径）。
  选型说明：播放方向已有 `PlaybackSink`、`Resampler` 是无状态变率原语，
  采集对齐需要水位伺服加欠载保持的有状态闭环，两者无法扩展，故单列文件；
  时钟域翻译仍是每方向唯一实现点，回调与处理循环内禁止另写对齐逻辑。

## 2026-09-02 — 播放时钟域收敛：唯一 PlaybackSink + 后端哑传输插件化（架构重构）

- **播放正确性收敛到唯一组件（重要）**：新增 `pvengine/dsp/playback.py`
  `PlaybackSink`——设备时钟唯一主时钟，全部跨时钟域消费（主输出/额外
  输出/网络输出/媒体从设备）统一经它：PI 伺服变速（±3% ASRC，积分收敛
  ~0.3s）消化速率差、预热水位、欠载静音重同步（绝不适配"复用上一帧"）、
  封顶丢最旧防延迟爬升、欠载边界 32 样本淡入淡出除咔哒。此前 5 条播放
  路径 4 种时钟策略（全双工内联处理/主输出帧长硬对齐/额外输出手写 ASRC/
  网络 drop-crossfade+pad/复用上一帧/Linux 无节奏 push+垫静音）收敛为
  一处；新增合成测试 `tests/test_playback_sink.py`（±2% 速率差长跑、帧长
  抖动、断流恢复、突发封顶全部不连续有界，CI 冒烟运行）；
- **传输后端真插件化（哑传输）**：Windows 新增 `pvplatform/audio/pa_backend.py`
  `PaBridge`（输入回调环 + 每输出设备一条独立回调流，回调按设备时钟经
  `out_pull` 拉 sink 帧，上混声道），与 Linux `PwBridge` 同形契约
  （open/read/close/active/set_far）；`audio_processor` 收敛为**统一处理
  循环**（本地/网络同一循环：read hop → process → sinks.write），删除全部
  5 套平台回调与平行缓冲（全双工单流内联处理、`_aligned_delivery`、额外
  输出 ASRC 结转、`_output_buffer` 网络环、"复用上一帧"）——播放正确性
  不再有第二实现点；输入与输出、主输出与额外输出改为地位对等的独立流
  （各自时钟域各自 sink），不再受 PortAudio 全双工单流同步抖动牵连；
- **修复 Linux 桥接依赖断裂（重要）**：旧桥调用的 pulsectl 流式 API
  （`connect_recording`/`connect_playback`）在 PyPI 全版本 pulsectl 中
  不存在（来自未记录 fork），干净安装（CI 打包的 deb/rpm/AppImage、内嵌
  python312）必现 AttributeError 起不来。改为自研最小绑定
  `pvplatform/audio/_libpulse.py`（ctypes 直调系统 libpulse：threaded
  mainloop + pa_stream 读写回调，播放 tlength 100ms/minreq 20ms、录制
  fragsize 20ms），**删除 pulsectl 依赖**（requirements 与关于页同步）；
  Linux 播放改为 libpulse 写回调按设备时钟 pull（无节奏 Python 轮询
  线程与垫静音循环删除），AEC far 流同构；
- **纯媒体会话多设备咔哒根除**：MediaSession 从设备由"队列扇出+欠载垫
  零"改为各自 PlaybackSink（设备间速率差变速消化），主从设备地位对等；
- 网络模式输出不再"缓冲 >60ms 丢帧"与欠载"复用上一帧"（冻结伪影），
  稳态漂移由 sink 伺服连续消化，acc 侧仅保留 80ms 硬顶兜突发；
- 处理循环健康诊断统一为一行（60s：fps/处理耗时/sink 最小水位/欠载·
  垫零·丢最旧），替代原回调诊断；
- TSE 参考录音钩子在 Linux 本地路径生效（此前仅 Windows 全双工路径接入）。

## 2026-09-01 — 10ms hop 规约全链对齐；修复 Linux 桥接数据丢失

- **修复 Linux 音频数据丢失（重要）**：PipeWire 桥接（pulsectl）数据粒度
  未随 202609 模型 hop 迁移同步，仍按 1024 样本分块、消费端按 480 读取，
  每帧丢弃 544 个样本（过半音频被丢、声音破碎）；桥接粒度统一为
  10ms（480 样本），采集/播放/AEC far 全部对齐；
- 平台层数据粒度统一 10ms hop：媒体会话混合粒度 1024→480（扇出队列
  深度保持 ~200ms），Windows loopback 缓冲注释按当前 hop 修正；
- **音乐播放器交互重做（无暂停/开始概念）**：播放开关 = 行启用复选框
  ——勾选原地续播、取消淡出停止，▶/⏸ 按钮删除；换曲/清曲在播放中
  也经包络淡出后切换（不再重启引擎、不打断麦克风链）；fx 行勾选改
  热更启停（此前整链 stop+start 重启会断麦，现仅音乐播放器/音效板/
  桌面声音等 fx 行原地生效，echo_cancel 仍走重启路径）；
- **修复参数滑轨量程**：滑杆被布局压窄后仍按请求宽绘制，正值区被裁
  到画布外（音量 dB 拉增「跑到不见了」）——改按实际渲染宽绘制；
  有符号量程（dB 增益）填充改从 0 位向把手延伸，默认值不再显示为
  「接近拉满」；
- **同类插件衔接问题同修**：音效板垫子起播/结束/手动停止/重播/垫子
  移除/复选框关断一律 hop 内线性淡变（此前垫子结束与手动停止为
  硬切咔哒）；桌面声音输入禁用时淡出并释放 loopback 捕获、勾回
  原地淡入（此前为瞬时旁路硬切）；
- **修复媒体衔接咔哒（撕拉）**：播放启停/seek/播放结束/缓冲欠载的
  状态切换原为硬切换（瞬时静音↔音乐的单点爆音），音乐混入改为每 hop
  ±1/n 线性淡入淡出包络，全部切换平滑无声（仿真实测启停跳变
  0.176 → 0）；
- **旁路全部对齐 10ms，无豁免**：浏览器采集 ScriptProcessor(1024) 改
  AudioWorklet（恒 480/帧切帧，输出静音不再本地回放麦克风）；频谱
  可视化窗 2048→960（=2×hop，FFT 无损窗长随 hop 派生）；音乐播放器
  解码读取 2048→1920（4 hop）；
- 主应用/lite_net 网络帧统一 480 样本（10ms，与引擎 hop 一致）；
  lite_net 抖动缓冲预填充保持 100ms 不变；浏览器/Android 调试面板
  帧大小显示同步更正；引擎冒烟（CI/本地）改按 hop=480 校验；
- 清理旧实现：引擎频谱 STFT 处理器（STFT 已在模型图内）、旧 960 帧
  PCM worklet（被 480 帧 AudioWorklet 取代）。
- **音乐播放器简化为硬切换 + 首尾相连环流**：曲目视为首尾相连循环流，
  放完即在解码器回卷@0 续流（环形缓冲无缝桥接，不停流、无静默衔接窗
  ——此前放完一遍要经排空/淡出/静默/重新淡入，衔接窗内只剩麦克风
  降噪本底、有细微嘶啦）；移除淡入淡出包络与「循环」勾选（必须循环），
  全部停止/开始/换曲为硬切换；行复选框关=硬停（管线旁路）、勾回原地
  续播；seek 逻辑不变；
- **修复物理输入与音乐同开的偶发弱咔哒**：全双工回调按 480 样本 hop
  处理后硬补零/硬截断到设备回调帧长——设备 frame_count 不恒等于 hop
  （WASAPI/MME 周期抖动），偏差帧在连续音频上挖洞/丢样本（纯麦克风
  不显、单开音乐走输出回调无此问题，物理输入+播放即闻"偶尔弱嘶啦"）；
  改为 hop 产出进输出累加器并保持 ~1 hop 交付滞后，按帧长取前缀、
  余量留待下次回调，帧长抖动不再挖洞不丢样本（本地媒体输出回调同修）；
- **额外输出跨设备时钟自适应（ASRC）**：主链与蓝牙等额外设备时钟存在
  ~±2% 速率差（实测主流回调节奏 98/100），此前按固定速率消费导致额外
  输出持续垫零（每 10 秒 ~6000 样本静音）即"开麦才有、与麦克风电平无关"
  的咔哒/破音；额外输出改为水位伺服的变率线性插值重采样（±3%），
  速率差被平滑消化，延迟稳定 ~50ms，音调恒定；启动预热 ~4 hop，极端
  漂移丢最旧防延迟爬升；回调健康诊断 60 秒一行（含字段判读，见日志
  `[诊断] 音频回调健康` 行）；
- **修复节点面板滚轮回调报错**：滚轮每次滚动抛 AttributeError——
  Canvas.yview_pickplace 在 Python 3.12 tkinter 已不存在（仅 Text 有），
  移除该无效调用；滚轮滚动行为保持不变。

## 2026-08-31 — 音量条数值/静音图标贴近滑条右端

- **VB-CABLE 音量条右侧标签贴近滑杆**：音量条右侧百分比/静音标签
  （vol_lbl）原用 `anchor="e"` + `width=8` + `padx=(pad_sm,0)`，文字右对齐
  在 8 字符宽标签内，短数值（如 `50%`）左侧留出大片死区，视觉上滑杆与
  🔇 静音图标/百分比距离很远。改为 `anchor="w"` + `width=7` +
  `padx=(2,0)`，文字从标签左边界起始直接贴近滑杆；width=7 仍容纳最长
  文本 `🔇 静音`（emoji 2 宽 + 空格 + 两中文 ≈ 6~7 字符）与 `100%`，
  不裁剪、不跳变，死区转移到标签右侧朝窗口边缘处不可见。静音/非静音
  态切换稳定。

## 2026-08-31 — 修复节点面板鼠标滚轮滚动崩溃

- **修复滚轮滚动时 `AttributeError: 'Canvas' object has no attribute 'yview_pickplace'`**：
  节点面板 `_wheel` 回调在已调用 `yview_scroll(-d, "units")` 完成滚动后，
  又多余调用了 `yview_pickplace("")`——该方法并非 Python Tkinter Canvas 的
  公开绑定（Tcl/Tk 虽有 `yview pickplace` 但参数形式不同且未被 Python 封装），
  触发属性错误导致滚动失效并向控制台抛异常。删除该冗余调用，滚动与边界
  夹紧完全由 `yview_scroll` 负责。长链场景（如 9 节点链）滚动恢复正常。

## 2026-08-31 — 收紧 HSlider 右端内边距

- **HSlider 把手与右侧数值间隙收紧**：自绘水平滑杆的右端内边距
  `_end_pad` 原为 `max(_hw + pad_sm, 12)`，基准缩放下 ctl_h=26 →
  _hw=8、pad_sm=4，恒为 12px；叠加 val_lbl 左侧 4px padx 后，最大值处
  把手到数值文字间隙偏空。改经两轮收紧：先到 `max(_hw, 8)`（基准 8px），
  再到 `max(_hw - 2, 6)`（基准 6px、大缩放跟随把手、极小缩放保底 6px），
  累计省 6px，把手与数值更贴近，仍保留不遮挡数字的最小安全距离。
  对所有使用 HSlider 的组件（参数节点滑杆、音量条、音乐播放器进度条）生效。

## 2026-08-31 — 修复音量条后台线程失效与硬编码设备名

- **修复音量条在实际运行时不显示**：根因是 `refresh_devices` 后台线程
  调用 pycaw 时未初始化 COM 套间，报 `OSError: 尚未调用 CoInitialize`
  被吞掉，导致 `_vb_cable_names` 始终为空集合、音量条永不显示。COM
  套间是线程级的（Tkinter 主线程自动初始化，后台线程不会）。现新增
  `_ensure_com()` 辅助函数，在每个 pycaw 入口（`_find_dev` /
  `get_vb_cable_names_win`）调用，用 thread-local 标记每线程只初始化
  一次 `CoInitialize()`（STA，与 pycaw 内部一致）。修复后后台线程
  `get_vb_cable_names()` 正常返回 VB-CABLE 设备名集合，音量条按预期
  显示与回显。同时为设备缓存加 `threading.Lock`（枚举在锁外、缓存
  读写加 double-check），保证主线程 `_viz_tick` 与后台预热并发安全。
- **消除硬编码 "CABLE Output" 字符串**：原 UI 层散布裸字符串
  `"CABLE Output"` 传入后端读写音量/静音，虽然后端按稳定规则匹配
  不依赖该名字，但可读性差且易误解为设备名匹配。现引入命名常量
  `CABLE_OUTPUT_KEY` / `CABLE_INPUT_KEY`（定义于 `_win.py`，经
  `pvplatform.system` 平台感知导出），后端 `_STABLE_RULES` 与 UI
  层所有调用方均引用常量，代码明确表达"这是逻辑键而非 FriendlyName"。
- **音量条标签动态显示实际设备名**：原标签写死 "CABLE Output 音量"，
  用户重命名设备后标签不更新。现新增 `get_cable_output_name()` 跨平台
  函数，按稳定规则定位录音端点返回其实际 FriendlyName，`_viz_tick`
  中动态更新标签文本（仅在变化时刷新，零额外开销），重命名后标签
  自动跟随。

## 2026-08-31 — 本地输出设备行新增 CABLE Output 音量条

- **音频输出行选 VB-CABLE 端点时显示 CABLE Output 录音端点音量条**：
  其他软件（Voicemeeter 等）的 AGC 会动态调整 Windows 录音端点
  `CABLE Output` 的主音量百分比，原界面无法直观看到该变化。现在
  「本地输出设备-音频输出」行内新增音量条，仅当所选输出设备为
  VB-CABLE 端点（含 "CABLE"）时显示，其余设备自动隐藏。
- **实时回显 + 可拖动调整**：音量条复用既有 HSlider 组件（深棕把手 +
  橙色填充，与全局滑杆风格一致），右侧百分比标签。33ms 周期复用 viz
  tick 读取端点音量回显滑杆，拖动时暂停外部读回防拽回；拖动过程只刷新
  百分比标签、不写 COM，松手时一次性写回端点主音量，避免 30+/s 写入
  卡顿。写失败时下个 tick 读回真实音量自然拽回滑杆。底层用 pycaw
  （Core Audio Python 包装，纯 Python）读写 endpoint master volume
  scalar，AudioDevice 按设备名缓存避免重复枚举；pycaw + comtypes +
  psutil 均为纯 Python、无自编译二进制，按 sys_platform 标记限定
  Windows 安装，符合项目硬约束。
- **跨平台抽象**：`pvplatform.system` 新增 `get/set_endpoint_volume_pct`、
  `get/set_endpoint_mute` 与 `invalidate_endpoint_volume_cache` 跨平台
  包装函数，非 Windows 优雅降级（返回 None/False），Linux 不受影响。
- **静音状态可视化**：端点被静音时，音量条把手与填充条统一变灰
  （TEXT_DIM/TEXT_FAINT），右侧百分比标签变为「🔇 静音」；33ms tick
  实时检测外部静音变化（如系统托盘或其他软件静音）自动回显。交互上
  支持点击百分比标签切换静音（Windows 音量惯例），拖动滑杆时自动取消
  静音——用户明确调音量即解除静音，无需额外点击。
- **抗设备重命名**：原实现按 FriendlyName 子串匹配（如 "CABLE Output"），
  用户在 Windows 设置里重命名设备后即失效。现改为按驱动描述
  （'VB-Audio Virtual Cable'，INF 写入不可改）+ 设备 ID 前缀
  （`{0.0.1.*}`=录音端点 / `{0.0.0.*}`=播放端点）稳定识别，重命名后
  仍正常读写音量与静音状态。UI 层同步改用 `get_vb_cable_names()` 返回
  的设备名集合判断是否显示音量条，不再硬编码 'CABLE' 子串。

## 2026-08-30 — AGC 节点实时增益改为可视化条

- **AGC 增益显示从文字标签升级为可视化增益条**：原文字标签因字体小、
  背景对比度不足，用户反馈看不清且难以感知变化。现改用 Canvas 绘制的
  水平增益条（uitk/viz.py 新增 AgcGainMeter），0dB 居中：
  向右绿色填充=放大（声音小时 AGC 提升增益），向左橙色填充=衰减
  （声音大时 AGC 压缩增益），条上叠加加粗大号数字显示当前 dB 值。
  ±30dB 范围与 AgcController 增益限幅对齐，快攻慢放平滑过渡防闪烁。
  引擎未运行或节点禁用时条内显示「AGC 未运行」提示。

## 2026-08-29 — 修复滑杆数值变化时滑条长度抖动

- **ParamSlider 数值标签固定字符宽度**：行内参数滑杆右侧数值标签（val_lbl）
  原无宽度约束，数值位数变化（如 AGC 目标从 `-12` 变 `-6`，两位变一位）时
  Label 宽度跳变，挤压/释放左侧 HSlider 的 expand 区域，导致滑条长度随数值
  不断抖动。现按 lo/hi 格式化后的最大字符数 + step 小数位 + 单位长度固定
  val_lbl 宽度（如压缩比 step=0.5 会产生 `10.5` 等带小数的中间值，已预留
  小数位），`anchor="e"` 保持文字右对齐；HSlider 可用宽度恒定，滑条长度不再
  抖动。对所有使用 ParamSlider 的参数节点（增益/AGC/压缩器/EQ/音效板等）生效。

## 2026-08-29 — 修复滑杆最大值处把手被数值遮挡

- **HSlider 宽度计算用反导致把手超出可见区域（根因修复）**：自绘水平滑杆
  `_val_to_x` / `_draw` / `_set_from_x` 三处都用 `max(winfo_width, self["width"])`
  取画布宽度。当 pack 布局压缩画布（如节点行内滑杆请求宽 250px 实际只分到
  220px）时，`max` 会取到过大的请求宽度 250px——把手被画到 220px 可见区域
  之外（数值标签下方），拖拽映射也基于 250px 计算，导致「滑块消失在数字背后
  还能继续往右滑」。改为优先用 `winfo_width()` 实际渲染宽度，仅在未映射时
  回退请求宽度。
- **HSlider 右端增加内边距，轨道与把手行程内缩**：同步引入 `_end_pad` 右端
  内边距（约一个把手宽度 + 间距），轨道右端、填充条右端、把手行程上界同步
  内缩，填充条自然缩短，最大值处把手与数值标签之间留出清晰间隙。拖拽映射
  公式同步调整，保证鼠标拖动与数值映射一致；该修复对所有使用 HSlider 的
  组件（参数节点、音乐播放器进度条）生效。

## 2026-08-29 — 加宽主窗口

- **主窗口基准宽度由 420px 加宽到 500px**：调整 `uitk/metrics.py` 的
  `win_w` 基准值（按分辨率挡位缩放），节点面板获得更多横向空间，参数滑杆、
  设备下拉与文本显示更宽松。高度不变。所有依赖 `win_w` 的布局（设备下拉宽、
  HSlider 默认宽、文本换行宽度等）自动按比例缩放。

## 2026-08-29 — 修复自绘滑杆把手在最大值处消失

- **修复 HSlider 把手与填充条同色导致最大值处视觉融合**：自绘水平滑杆
  （`uitk/widgets.py` 的 `HSlider`）把手原用 `ACCENT`（南瓜橙）填充，与
  已填充轨道段同色。当滑杆值到最大（如 AGC 目标 -6dBFS）时整条轨道被橙色
  填满，把手仅靠 1px 深棕描边区分，视觉上几乎「消失」。改把手为 `TEXT`
  深棕填充 + `ACCENT` 橙色 2px 描边——深棕在橙色填充条与浅棕轨道上均有
  高对比，两端都清晰可见；该修复对所有使用 HSlider 的参数节点
  （增益/AGC/压缩器/EQ/音效板等）生效。

## 2026-08-29 — 修复自动增益 AGC 节点静默失效

- **修复 AGC 节点带参数时创建失败被静默吞掉的问题**：`AgcPlugin.__init__`
  原先调 `super().__init__(params)` 再建 `self.agc` 控制器；基类在 params
  非空（用户调过目标 dB 并保存配置）时会触发 `on_params_changed()` 访问
  尚未创建的 `self.agc`，抛 `AttributeError` 被 `set_plugins` 的 except 吞掉，
  表现为 AGC 节点加入链后完全无效。改为先建控制器再调 super（与
  GainPlugin / CompressorPlugin 同模式）；
- **修正 AGC 时间常数与实际帧长不匹配**：`AgcController` 原用默认
  `call_interval_ms=10ms` 计算 attack/release/rms_ema，但实际帧长为
  1024@48k ≈ 21.33ms，导致全部包络响应偏慢约 2 倍。现按真实帧时长传入，
  AGC 收敛速度符合设计预期。

## 2026-08-29 — 应用图标重设计：像素字体「P」单一绿色，取消运行/停用双态

- 应用图标与 Lite 版同源：ark-pixel 像素字体「P」（深绿字身 + 亮绿描边）
  程序化生成，托盘/任务栏/EXE 图标统一为单一 `audio_icon.ico`
  （16~256 多尺寸帧），不再区分运行/停用双色——删除
  `audio_icon_on.ico` / `audio_icon_off.ico`（托盘本就只显示其一）；
- Linux 打包（deb/rpm/AppImage）桌面图标改由 `audio_icon_base.png`
  512px 基图直接缩放，不再从 ICO 抽帧。

## 2026-08-28 — 媒体播放收敛 miniaudio 单路径；不支持格式选曲时自动转码

- **播放改用成熟播放库直出**：纯媒体会话（无麦克风输入）弃用自研
  「泵线程 + 环形缓冲 + PyAudio 回调」，改 miniaudio PlaybackDevice
  拉模型——播放库设备回调直接拉混合帧，设备时钟即节拍；多路输出由
  主时钟设备经有界队列扇出（满丢最旧/欠载补静音）；输出设备按名称
  精确匹配，未选/未匹配 = 系统默认输出；
- **解码单一路径 = miniaudio**（wav/mp3/flac/ogg），删除运行时
  PyAV/wave 回退链；选曲/选音效时做格式归一：不支持的容器/编码
  （m4a/mp4/aac/wma/opus…）用 PyAV 一次性转码为「原名.purevox.wav」
  （48kHz 单声道 16bit WAV）并自动改用转码后的文件名——运行时永远
  只走 miniaudio，无静默回退；同名转码产物未过期时直接复用；
- seek 改 miniaudio 原生 seek_frame 定位（C 级 ma_decoder），不再
  重开 PyAV 容器；
- **修复「桌面声音输入」节点从未生效**：导入的 SpeakerCapture 符号
  不存在（ImportError 被静默吞掉，节点恒直通无声），改走
  create_speaker_capture 工厂；Linux 未提供会话桥时自建独立 PwBridge
  （AEC 传桥路径不变）；
- 中文/非 ASCII 文件名全链路适配：音效板解码改内存路径（绕开
  miniaudio 在 Windows 的 ANSI 路径限制），选曲转码与流式播放本就
  宽字符安全；
- VU 电平条改快攻慢放（上升瞬间到位/下降按 90dB/s 滑落），且空抽头
  保持当前电平（拉模型突发供帧不再打回零）——电平是「动」不是「闪」。

## 2026-08-27 — 音乐播放器 v2：流式解码 + 进度条 + 断点续播

- **流式解码取代整曲入内存**：后台解码线程逐包解码（PyAV）→ 重采样 →
  3 秒环形缓冲，音频线程只读缓冲——内存占用恒定 ~0.6MB，不再随文件
  时长线性增长（长视频 mp4 实测 32 分钟文件内存零增长）；
- **进度条滑块一体**：拖动即定位（seek 容器级微秒单位，修复此前 seek
  单位错误导致「永远从头/拖动卡死/进度条不可用」），播放中实时回显；
- **单键播放/暂停**（▶⏸ 随状态切换），移除独立停止键；
- **断点续播**：位置经 params.resume_sec 在暂停/拖动/退出时事件触发
  持久化（粗粒度），重启后首次播放自动续到该处；
- **启动/seek/开关插件防瑕疵**：0.25s 预填充门——解码灌到水线前音频面
  吃静音，不再把空缓冲的零帧周期性灌出（爆音瑕疵根除）；
- 修复解码线程非重入锁自死锁（首次写环形缓冲即冻结）。

## 2026-08-27 — 纯媒体会话：无麦克风输入也可用（文件即输入）

- 会话计划放行「仅媒体源」：启用中的音效板/音乐播放器/桌面声音输入本身
  即可作为输入源，不再强制要求麦克风输入节点（禁用全部媒体后仍会阻断）；
  无麦克风时提示「本次仅媒体源发声」；未选输出设备 = 系统默认输出；
- 媒体播放走**独立 MediaSession**（pvplatform，自含输出路径）：
  生产泵 → 2s 环形缓冲 → RT 回调逐帧消费，欠垫复用上一帧平滑、
  200ms 预填充、450ms 高水位——播放速率由硬件消费端定义，
  主体传输层（AudioThread）零改动、零掺杂。

## 2026-08-27 — 媒体格式面拓宽到 Soundpad 同级；修复 miniaudio 整数域削波

- **解码统一三段回退链**（新工具模块 audio_decode，音效板/音乐播放器共用）：
  miniaudio（wav/mp3/flac/ogg）→ PyAV（m4a/aac/wma/opus 与 mp4/mov/webm/mkv
  等容器音轨）→ wave 标准库兜底；文件对话框同步放宽，mp4 等视频容器直接
  取音轨当音源；
- **修复 miniaudio 解码削波**：其默认输出为 s16 整数域（幅度 32767 量级），
  直接当浮点用会严重削波；现显式指定 float32/单声道/48k 输出；
- **PyAV 转全平台安装**：Linux 主线同样获得长尾格式解码（包体 +~30MB）。

## 2026-08-27 — 新增音乐播放器与桌面声音输入（两个独立媒体节点）

- **音乐播放器**：选曲目（mp3/flac/ogg/wav，miniaudio 解码整曲入内存，
  WAV 标准库兜底），▶/⏸/■ 与循环开关，音量滑杆实时生效；
- **桌面声音输入**：系统混音 loopback 接入处理链（复用 AEC 的平台捕获
  工厂：Win=WASAPI loopback / Linux=PipeWire monitor），随引擎启停
  自动开关，音量滑杆实时生效；
- 两者与音效板相互独立、可任意组合多实例；统一挂「添加 ▾ → 媒体输入」
  分类；位置语义同音效板（默认链尾直通，可拖动参与处理）。

## 2026-08-27 — 新增音效板（Soundpad 类媒体输入节点）

- **添加 ▾ → 音效板（媒体输入）**：垫子 WAV 懒加载（8/16/24/32bit、
  多声道自动下混、任意采样率重采样到 48kHz），行内点击 ▶/■ 播放/停止、
  「全部停止」、音量 dB 滑杆实时生效；垫子随链配置持久化，运行中增删
  即时生效无需重启；
- **全局热键**：每垫勾选即绑定 Ctrl+Alt+1..9（Win32 RegisterHotKey
  message-only 窗口事件驱动，无轮询；非 Windows 静默降级为仅按钮）；
- **链位置语义**：默认追加链尾 = 后级直通——音效不被降噪/变声，随全部
  输出扇出；可拖至降噪之前参与处理。与多路输出/输出位置抽头天然兼容；
- **多路输入现状说明**：Windows 传输层仅声明多路输出能力（多输入设备级
  混音为 PipeWire 独占），音效板以进程内注入补齐跨平台「设备外输入」。

## 2026-08-27 — 修复多路输出失效（额外输出设备被硬编码丢弃）

- **修复 full 版多路输出扇出失效**：引擎启动时额外输出设备 ID 列表被写死
  为空，除第一个输出设备外全部无声（Linux PipeWire 路径不受影响）；
  现按节点行顺序把其余输出设备全部接入扇出，任一设备打开失败仅跳过
  该设备不阻断；多输入在 Windows 的设备级混音为传输层既定缺口
  （Linux PipeWire 多输入正常）。

## 2026-08-27 — Lite 托盘修复：菜单构建即误触发缩放；图标固化为仓库资产

- **修复 Lite 冻结版启动挂死（表现即「稳定没有托盘」的根因）**：像素字体
  注册后用同步 `SendMessage(HWND_BROADCAST, WM_FONTCHANGE)` 广播，会被任意
  一个不泵消息的顶层窗口无限挂起，UI 与托盘双双无法创建；改用异步
  `SendNotifyMessage`，本地冻结双包实测托盘与主窗齐活；
- **修复右键托盘即字体缩放七连闪、选档无效**：菜单构建时回调被多余的一对
  调用括号立即执行（pystray 迁移遗留），每次弹出菜单就把全部 7 个缩放档位
  连发投递进主线程，字体因此抖动、最终档位随机；回调现仅在真正点选后执行；
- **托盘/窗口/exe 图标统一为仓库资产**：`assets/icons/lite_tray.ico`
  （16~256 全帧梯，像素字体 P 最近邻采样）与 `lite_tray.png`（64px 母版）
  入库，开发态直接读文件、PyInstaller 随包携带、exe 图标同一文件——
  运行时不再依赖字体绘制，任意 DPI 下 1:1 渲染零重采样；
  重设计请手动重跑 `tools/gen_lite_tray_icon.py`；
- **依赖清单收敛单文件**：requirements-win.txt 并入 requirements.txt
  （平台差异用 `sys_platform` 环境标记），两条 Windows 工作流轮子集与
  pip 缓存键完全一致，缓存直接互通。

## 2026-08-27 — Lite 与主线同套依赖；Windows 构建剔除 PySide6 残留

- **Lite 依赖对齐主线 pin 版本**：onnxruntime 升至 1.29.0、numpy/scipy 与主线
  完全一致，Lite Net 新增组件（PyAV 18 / websockets 17 / qrcode 8）一并 pin 并
  以环境标记仅作用于 Windows——Linux 主线安装清单不变。两个 Windows 工作流
  （主线 / Lite）从此共用同一轮子集，pip 缓存桶直接互通；
- **修复 Lite 构建引用已删除目录**：字体收敛到 assets/fonts 后打包脚本残留
  `lite_mic/fonts` 引用，全新环境构建必失败；现与主线同映射，冻结态字体
  定位统一走主线多根探测函数；
- **Windows 主程序产物剔除 PySide6**：系统 accent 取色是纯 Qt 时代残迹且
  已无调用方，连同一处指向旧入口的自启动命令一并移除，产物体积明显下降。

## 2026-08-27 — 托盘健壮性三端统一：创建即校验、丢失即事件自愈、关窗跟随真实状态

- **修复轻量版偶发「无界面僵尸进程」**（与主程序同源问题）：旧实现托盘
  成败只看「库是否导入成功」，图标线程内部失败不可见；此时关闭窗口仍按
  「隐藏到托盘」处理，进程活着却永远无入口，还占住单实例锁导致下次启动
  报「已在运行」。现三端（完整版 / Lite / Net Lite）共用同一套零依赖
  Shell_NotifyIcon 托盘原语，三条原理级保证，无看门狗、无定时器、无延迟
  重试：
  - **创建即校验**——图标添加结果在创建返回前同步确认；添加失败即按
    「无托盘」运行，关闭窗口直接干净退出；
  - **事件驱动自愈**——explorer 重启清空任务栏时系统广播 TaskbarCreated，
    收到即时重建图标，中丢不再需要重启软件；
  - **策略跟随状态**——「关闭窗口=隐藏到托盘」仅在图标确实存活时生效。
- **Lite 减少一个第三方依赖**：两个轻量版弃用 pystray，改用与完整版相同
  的 ctypes 实现；托盘菜单功能不变（显示主界面 / 缩放比例勾选档位 /
  退出），打包体积相应减小。

## 2026-08-26 — 修复 Linux 未注册内置像素字体（无 CJK 字体的系统中文豆腐）

- **内置 Ark Pixel 字体改为跨平台注册**：此前仅 Windows 生效（GDI 私有
  加载），Linux 从不注册、且 deb/rpm/AppImage 包内根本没带字体文件，
  系统缺中文字体时界面中文显示为豆腐块。现字体收敛到仓库唯一副本
  `assets/fonts/`（删除 lite_mic/lite_net 各自的重复副本），Linux/macOS
  运行时经 freedesktop 用户字体目录（`~/.local/share/fonts/purevox`）+
  `fc-cache` 注册，无需 root、不污染系统字体；deb/rpm/AppImage/Windows
  打包均随包携带。附带修复：Windows 打包版此前因字体文件未进产物而一直
  静默回退系统雅黑，现像素字体真正生效。

## 2026-08-26 — 修复 RPM 包未捆绑运行时（体积异常小且装完无法运行）

- **RPM 与 deb/AppImage 对齐为同一实现路径**：`pack_rpm.sh` 此前只打包
  源码+模型（约 13MB），启动脚本直接调系统 `python3`，numpy/onnxruntime 等
  依赖要求用户自行解决（Fedora PEP668 下 pip 安装被系统拒绝），
  装到干净系统上无法启动。现改为捆绑内嵌 Python 3.12（与 deb/AppImage
  完全一致，全部 Python 依赖随包携带），启动脚本经 PYTHONHOME 使用包内
  解释器；Requires 保持仅 pipewire/opus。CI 的 fedora job 同步补
  wget/ImageMagick 并共享 ~/.cache/purevox 缓存。

## 2026-08-26 — 清理历史兼容层：删除 aimic 垫片与引擎旧接口残留

- **删除 `aimic.py` 兼容垫片模块**：纯 Python 引擎（pvengine）自 2026-08-22
  接管全部实现后，该文件仅剩旧模块名转发；唯一在库调用方已改为直接导入
  pvengine，打包脚本与 CI/本地引擎冒烟同步切换。
- **删除引擎旧接口兼容垫片**：AudioProcessor 上无调用方的旧 setter/getter
  （set_mode/get_mode/set_pre_gain/set_agc_enabled/set_vad_enabled/
  set_compressor_enabled/set_io_sample_rates/is_aec_available/is_tse_available/
  set_aec_far_rms_target 等）与恒定值的 backend_effective/backend_reason/
  backend_info 后端报告一并移除；运行中参数热更统一走 update_plugin_param，
  构造函数不再接收被忽略的模型路径参数（模型路径由插件按 model_config 解析）。
- **清理其余兼容残留**：日志器旧式 `log()`/`Logger.__call__` 标签自动识别入口、
  Resampler 构造函数中被忽略的 converter_type 参数、uitk 主题 ALT_BASE 旧别名
  （统一为 PANEL）、lite_mic 中无人调用的 list_devices_compat；
  第三方许可证文档移除已不再随包分发的组件条目（PySide6/libsamplerate/pffft/
  7-Zip）。

## 2026-08-25 — Linux 内嵌 Python 改用预编译包，bootstrap 不再编译

- **bootstrap_python312.sh 从源码编译改为预编译分发**：改下载
  python-build-standalone（Astral/uv 生态）的 CPython install_only 包，
  解压即得完整解释器（含 ssl/_ctypes/pip），内嵌 Python 版本由 3.12.11
  升至 3.12.14。首次准备时间从数十分钟编译缩短到一次约 30MB 下载；
  CI 的 AppImage job 不再需要 libssl-dev/libffi-dev/zlib1g-dev/build-essential。
  离线环境仍可用 `PUREVOX_CPYTHON_TARBALL` 指定本地包。

## 2026-08-25 — CI 与本地构建对齐：Linux 依赖改用 requirements.txt，新增本地全流程脚本

- **CI Linux 依赖改为 requirements.txt 同源安装**：此前 Linux job 用临时
  pip install 列表装最新版 numpy/onnxruntime 等，与本机 pin 版本不一致，
  存在产物行为漂移风险。现与 Windows job 一致，从 requirements.txt 安装
  （pillow 仍单独安装，仅打包脚本需要）。
- **新增本地全流程脚本**（无需推 tag 即可复现整条 CI）：
  `verify-local.sh`（Linux 内跑 Linux job 全套——系统依赖、依赖安装、引擎冒烟、
  deb/AppImage/rpm 打包；在 vboxsf/9p 共享目录上运行时自动切到原生文件系统
  构建，规避其不支持软链的限制）；`verify-local.ps1`（Windows 入口，一条命令依次
  经 WSL(Ubuntu-24.04) 跑 Linux 段、本机跑 PyInstaller 打包与 Android APK）。

## 2026-08-25 — 修复设置菜单与 VB 驱动卡片点击无效，「启动时自动运行」落地

- **修复「系统声音」点击无反应**：日志器误写成构造函数内的局部导入，
  菜单回调里引用不到即抛异常又被静默吞掉。改为模块级导入后，
  「系统声音」（Windows 声音控制面板）恢复正常。
- **修复 VB-CABLE 卡片「打开控制面板」无反应**：Tk 版迁移时遗漏了该动作的
  平台接口导入（NameError 被吞）。现已接通：已安装驱动时点击经 UAC 提权打开
  VB-CABLE 控制面板。
- **修复「开机自启」勾选无效**：同一局部导入问题导致注册表写入从未执行，
  现已恢复（Windows 经 UAC 写 HKLM Run 键）。
- **「启动时自动运行」勾选后真正生效**：此前只保存配置、无人消费。
  现对齐旧版行为——下次启动不再弹主窗、直接进托盘，约 1 秒后自动开始
  音频处理；托盘或快捷键可随时唤出窗口。
- **「快捷键」开关生效**：取消勾选后，右 Alt + > 不再触发窗口显隐。

## 2026-08-25 — 设备扫描只在启动与启停时进行；退出按钮改为红色

- **移除下拉展开触发的设备刷新**：运行中引擎占用 PyAudio，此时枚举必然失败，
  打开设备下拉反而拿到空列表/旧数据。现在设备列表只在两个时机后台重扫——
  程序启动、点击「启动/停止音频处理」；VB-CABLE 状态卡同步改为这两个时机检测
  （卡片提示文字已更新）。
- **退出按钮改红色实心底**（与停止态同色系），文字颜色随其余按钮统一。

## 2026-08-25 — 桌面端全面切换纯 Tkinter UI（PySide6 版本移除）

- **run_tk.py 成为唯一桌面入口**：删除根目录全部 PySide 界面文件
  （ui_pyside6/run_pyside6/dialog_* 五件套/spectrum_histogram/theme_colors），
  历史版本已归档于 legacy 快照；关于页文本迁至无 Qt 依赖的 about_content.py；
- **依赖瘦身**：不再需要 PySide6（约 170MB），requirements 与打包产物同步移除；
  Windows 包体积显著下降，启动不再加载 Qt；
- **功能对齐快照**：关于页补齐「关于 / Windows 使用 / Linux 使用 / 更新日志 /
  许可证」五个标签（含第三方库清单与链接）；虚拟输出行的 VB-CABLE 卡片完整
  实现原弹框内容（状态灯自动刷新 / 双端点说明与数据流向 / 驱动卡片：
  打开控制面板·下载官方驱动包·安装视频教程 / 启动检测开关）；修复「系统声音」菜单。

## 2026-08-25 — 移除创意音效，均衡器改为标准 1/6 倍频程图示 EQ 做法

- **删除全部 13 个创意音效节点**（混响/延迟回声/合唱/镶边/移相器/颤音/
  自动哇音/失真/比特粉碎/激励器/噪声门/限幅器/电话声效）：PureVox 是
  麦克风工具，不需要吉他踏板类效果。其中——
  噪声门/限幅器与核心插件重复（噪声门 VAD、压缩器已覆盖，单一路径原则）；
  电话声效本质是 300~3400Hz 带通、激励器的音色作用是高频提亮，
  两者均衡器即可实现；其余为纯创意效果，整体随 fx 包删除。
  强配置不迁移：旧链中残留的音效条目直接丢弃，不做兼容；
- **均衡器修正为标准做法**：61 段频点为 ISO 1/6 倍频程栅格
  （20 Hz~20 kHz），峰值滤波器 Q 由固定 1.41（1 倍频程带宽，系旧版
  少段位遗产）改为按栅格带宽匹配的 ≈8.65
  （Q = √(2^N)/(2^N − 1)，N = 1/6）——每段 −3dB 带宽≈本段栅格宽度，
  拖动一段只影响附近频带，不再向邻段大面积串扰、多段同抬不再低频糊成一团；
- **EQ 性能与状态**：零增益段运行时直接跳过（恒等滤波器跳过不改输出，
  典型场景 CPU 大降）；新激活段滤波器状态清零，避免旁路期间残留历史产生瞬态；
- **响应曲线单一来源**：全链频响计算收敛到引擎 `response_at()`，
  曲线编辑器与引擎共用同一份 RBJ 系数与 Q，显示即所得；
- **均衡器新增高切/低切**：面板复选框 + 截止频率（低切默认 80Hz、高切默认
  16kHz，默认关闭），二阶巴特沃斯 12dB/oct 标准做法，信号流为
  低切 → 峰值段 → 高切；设置随配置持久化，两种 UI（PySide/Tk）均支持；
- **Tk 编辑器升级为真实 61 段**：旧版是「10 手柄对数插值」的代理视图
  （仅到 16kHz），现直接展示引擎全部 1/6 倍频程频点（20Hz~20kHz），
  拖拽/滚轮即改对应频段，曲线为引擎真实合成响应；
- **设备下拉展开即刷新**：点击任意设备下拉（输入/输出/回声参考）都会重新
  枚举设备——插入新设备后打开下拉即可选到，无需重启软件；与启动/停止/
  弹框后的既有刷新共用同一套后台枚举逻辑，快速连开下拉自动丢弃过期结果。

## 2026-08-24 — 修复首次启动偶发"无界面僵尸进程"占用单实例锁

- **托盘无条件创建**：不再以 `isSystemTrayAvailable()` 为前置条件
  （explorer 托盘未就绪时该检查偶发 False，跳过建托盘会导致
  进程在后台运行却永远没有图标，再次启动提示"已在运行"）；
- **无托盘时不隐藏式关闭**：关闭窗口改为"有托盘才隐藏到托盘，
  无托盘直接退出"，杜绝界面不可见但进程残留；
- **启动看门狗（20 秒）**：启动后既无可见窗口也无托盘即记错误
  日志并强制退出，释放单实例锁，用户可立即重新打开；
- **分阶段启动日志**：配置/日志/主窗口/UI/托盘各阶段写入日志文件，
  便于定位启动卡点。

## 2026-08-23 — 应用图标改为像素画「P」：莫兰迪同色配色 + 全像素风缩放

- **状态色区分加强**：运行/停用两态色相拉开（绿 `(74,142,96)` /
+  红 `(206,104,92)`），字身提浅比例降至 0.15，避免浅调冲淡色相；
 - **底稿更换**：32×32 像素画（外部像素编辑器导出后内嵌为常量，
  零外部素材依赖），删除旧 ark-pixel 细分格灯管绘制算法；
  描边按运行状态切换（灰绿=运行 / 灰红=停用），字身为描边色
  向白提浅的同色系浅调——外深内浅、莫兰迪低饱和
- **落点**：按包围盒居中后，左右/上下余量的 55% 分给左上，
  视觉中心微偏右下（修正右上视觉重）
- **全像素风缩放**：32/64/128/256/512 严格整数倍 1:1 最近邻放大；
  16/20/24/48 按覆盖面积做两级多数投票降采样（先决不透明保剪影，
  再定描边/字身），只输出调色板原色或全透明——无混色、
  无半透明、无次像素渲染

## 2026-08-23 — 桌面 UI 整改：单一墨黑主题、顶栏三行合一、节点行拖拽排序

- **移除主题切换**（系统/白天/黑夜三选一）：桌面端收敛为单一墨黑深色主题，
  删除 theme_colors.py 的浅色平行定义与 config 的 theme 键；高亮色仍跟随
  系统 accent，标题栏恒深色
- **顶部三行合一**：原生菜单栏（设置/系统声音/虚拟声卡/关于）与面板头
  （添加下拉/清空）并入单一工具条 = 启动 · 退出 · 添加节点▾ · 清空 · 设置▾；
  设置菜单收纳快捷键/自动运行/开机自启勾选项与关于入口
- **节点排序改拖拽**：删除上移/下移按钮，行首拖拽手柄按住即可拖动排序
  （浮层跟随 + 实时落位）；删除按钮换用 PIL 生成的 close 图标（悬停红色）
- **节点行视觉简化**：圆角卡片改为扁平行 + 发丝分隔线 + 悬停底色，
  更紧凑、层次更清晰
- **修复横向滚动**：面板禁横向滚动条，设备下拉可压缩（不再把内容撑宽）
- **应用图标重绘**：字形取自 ark-pixel「P」（内嵌常量，碗形封闭），
  细分格算法原分辨率直绘——实心 P 每设计格细分 3×3，剪影外扩 1 格
  作环（厚恒 1 格、全直角），所有转折交叉角置空；字形放大充满画布，
  ≤16px 托盘自动改内描边顶满画布；**修复 ICO 只含 16px 单帧的问题**
  （基底改用最大帧，此前大图标由 16px 硬放大导致发糊），现含
  16~256 八档逐尺寸帧

## 2026-08-23 — 仓库结构整理：模型/图标/工具归位，命名清晰化

- **models/**：根目录三个 ONNX 模型（降噪/AEC/TSE）归入 models/，
  仓库与所有打包产物（deb/rpm/AppImage/PyInstaller）统一该布局
- **assets/icons/**：应用图标（on/off ico 与 512 基图）归入 assets/icons/
- **Lite 应用改名**：lite_denoise_only → lite_mic（PureVox Lite）、
  lite_net_only → lite_net（PureVox Net Lite）；构建脚本对齐为
  build_lite_mic.ps1 / build_lite_net.ps1（原 build_lite_local/build_net_local）
- **tools/ 收敛**：scripts/slim_pyside6.sh 与根目录 diagnose_wasapi.py 并入 tools/；
  删除零引用残留 sal_fix.h、sfx_config.txt（旧 C 构建 / 7z SFX 打包时代产物）
- 纯重命名与引用同步，无功能变化

## 2026-08-23 — 移除 cpython git 子模块，引导脚本按需下载源码包

- **packages/cpython 子模块整体移除**：Linux 内嵌 Python 3.12 改由
  bootstrap_python312.sh 按需下载官方 CPython@v3.12.11 tarball 后一次性编译
  （缓存于 ~/.cache/purevox，PUREVOX_CPYTHON_TARBALL 可指定离线包），
  克隆仓库不再需要 submodule 步骤
- CI 缓存 key 由「子模块 SHA」固定为 cpython 版本号；checkout 不再拉子模块

## 2026-08-23 — 传输后端插件化（架构升级 · 第一阶段）

- **平台音频 API 成为可插拔后端**：新增 pvplatform/audio/backends.py——
  BackendSpec（名称/显示名/平台/能力集/优先级）+ 探测 + 选择器；
  内置 pipewire / wasapi / mme 三个后端，能力声明
  （multi_input/multi_output/loopback_far）取代隐式平台假设
- 启动流程按「平台→探测→能力覆盖」自动选择唯一后端并写入启动日志；
  能力不足时明确报错（如 Windows 上启用两个输入节点会提示该后端不支持多输入）
- DESIGN.md §5 重写为传输后端规范：数据面契约与 PwBridge 同形，
  禁止传输代码散布 if IS_LINUX；Windows 回调数据面的完整类化提取为 TODO v2

## 2026-08-23 — 启动闪退修复 + 全组件压测工具（稳定性）

- **修复启动即闪退**：历史重构误删的模块常量（_VU_TICKS / _VU_GREEN / _VU_YELLOW /
  _VU_RED / _LAST_48K_WARN）补齐——首次绘制抛异常在真实窗口模式下直接中止进程；
  离屏冒烟不触发绘制所以此前未暴露。教训入册：常量删除必须全文件引用扫描
- **新增 tools/smoke_all_components.py**：全组件压测——注册表内每个节点类型全部
  入链（含双输出扇出），offscreen 渲染 2 秒 + SessionPlan 校验 + 配置往返，
  --real 追加真实设备满链启动（实际开流跑 2 秒后干净停止）
- 实测通过：Windows 满链（21 处理级 + 双输出扇出 + AEC far 采集 + VU/频谱）
  启动、运行、停止全程零异常

## 2026-08-23 — 顶层设计规范落地：节点注册表统一 + 会话计划层（架构升级）

- **DESIGN.md 新增**：分层架构（L0 平台 → L1 引擎 → L2 传输 → L3 会话 → L4 UI）、
  节点模型规范、数据流不变量（F32 单声道 48k / 等权混音 / 扇出互不拖累 / viz 只读旁路）、
  SessionPlan 契约、错误降级矩阵、扩展清单——实现与规范的冲突以 DESIGN.md 为准
- **节点注册表统一**：pvengine/plugins.py 引入 `NodeSpec`（name/label/kind/tier/params），
  fx 插件由类属性自动派生，系统节点显式注册；发现入口收敛为
  `all_specs()/get_spec(name)`，UI 与会话计划禁止自建类型清单
- **新增 session_plan.py（L3）**：`SessionPlan.from_chain()` 纯函数把链文档校验为
  可执行计划（inputs/outputs/remote_url/viz/fx_chain + 阻断 problems/非阻断 warnings）；
  启动流程不再内联解析链配置
- **修复**：PluginPanel 保存调用不存在的 `DebouncedSaver.schedule` → 统一为
  `request_save`；remote_mic 行体分支顺序错误导致 URL 输入框不可达 → 修正优先级；
  viz 控件注入引用失效布局 → 改持卡片布局引用
- 清理死代码：`_combo_value`/`restart`/`stop_processing_for_update` 及 MainPanel 残留

## 2026-08-22 — 全插件化音频链 + 主界面双栏重构（重大更新）

- **固定模式取消，全部处理插件化**：直通/降噪/AEC/TSE 四档模式选择器移除，
  前增益/AGC/VAD/均衡器/压缩器/AI 三件套与全部音效统一为**可自由编排的
  插件处理链**——右侧面板添加、删除、上下排序，顺序即信号流
- **主界面双栏布局**：左列自上而下 输入/输出/监听/启动控制/VU 电平表/频谱图；
  右列为处理链面板。频谱宽度自适应（原固定 551px）
- **三级插件 UI**：
  - 基础（开关）：AI 智能降噪等无参插件仅一行开关
  - 中级（行内控制）：增益滑杆、压缩器阈值/比率/补偿、回声消除的 far 端扬声器
    设备下拉（替代原 AEC 模式复用监听下拉的设计）等
  - 高级（展开独立 UI）：均衡器行「均衡器…」弹出曲线编辑器（含预设），
    TSE 行「参考音频…」弹出录音对话框；混响/延迟/合唱/镶边/移相器/颤音/
    自动哇音/失真/比特粉碎/激励器/噪声门/限幅器/电话声效共 13 个新音效为中级行内控件
- **实时调参**：处理运行中拖动滑杆即时生效（不重建链、不断流式状态）；
  结构变更（增删/排序）整链热替换；全部配置持久化到 plugin_chain 键

## 2026-08-22 — 引擎全面纯 Python 化：组件化架构（重大重构）

- **删除全部 C 代码与自编译二进制**：`aimic.c`/`aimic.h`（C DSP 核心）、
  `pipewire_client.c`（PipeWire 桥）、`alsa_client.c`（ALSA 桥）、`pvpipe.py`/
  `pvalsa.py`（ctypes 绑定）、`setup.py`（gcc 构建链）、捆绑的 onnxruntime
  预编译 SDK（win/linux 双份）全部移除。项目不再需要 gcc/mingw，CI 删除
  msys2 与 C 编译步骤
- **新增 `pvengine` 纯 Python 组件化引擎**：Stage 接口（process/reset/release）
  为组件唯一契约，组件按 active_modes 声明生效模式，可随意增删替换重排——
  components/ 下 denoise/aec/tse/gain/eq/vad/agc/compressor/clip/recorder/tap
  每个文件一个组件；dsp/ 提供窗函数/STFT/环形缓冲/流式重采样/Mel 频谱等可独立
  复用的基础件；pipeline.py 按序执行 + 模式旁路。numpy 负责帧级 DSP，
  scipy 提供 EQ 双二阶层联（lfilter），onnxruntime 跑模型推理
- **三个 ONNX 模型逐位语义移植**：v9 降噪（spec [1,1025,1,2] interleaved +
  enc/dec/tfa/inter 四状态，sqrt-Hann、OLA 归一化阈值 1e-6、3 帧静音预热）、
  aec9（mic/far planar 谱 + 全套流式状态 + far 端非 48k 时内部重采样）、
  tse15（spec_frame/enr_spec/cache 契约、参考音频镜像填充逐帧 STFT 缓存）。
  与 lite 引擎同模型对比验证，输出差异仅浮点噪声级
- **Linux 音频桥改 pulsectl**：ctypes 到系统 libpulse，走 pipewire-pulse 兼容层；
  每条流独占线程 + 独立 Pulse 连接。**ALSA 备选接口整体移除**（旧混合实现连同
  UI「本地接口 ALSA」选项），单一路径 = pipewire-pulse
- **viz 内存隐患根治**：旧 C 版 process_pipeline 无条件向可视化缓冲追加且只增不减
  （~1.4GB/小时）；新版改为有界 BufferTapStage（超限丢最旧）且仅网络管线内临时启用
- **依赖变更**：requirements.txt 新增 numpy / scipy / onnxruntime / pulsectl
  （不锁版本，安装即最新）；deb Depends 只留 pipewire（去 libasound2）
- **`aimic.py` 变兼容垫片**：re-export pvengine，AudioProcessor/RingBuffer/
  Resampler/compute_spectrum 等 API 不变，调用方零改动

## 2026-08-22 — Lite 网络模式：切换网络稳健性修复与逻辑精简

- **防火墙零逻辑**：删除全部主动防火墙代码（旧端口级规则/UAC 提权安装/规则检查/
  防火墙按钮与提示）。放行唯一路径：WSS 开始监听即触发系统「安全中心警报」，
  点允许即生成按程序放行的规则；「重启」按钮重开监听会再次触发警报，无需任何
  检查或安装代码
- **新增「重启」按钮**（二维码左侧）：WSS 重开监听 + mDNS 重注册 + 网卡列表/
  服务状态刷新的统一手动恢复入口
- **修复开机自启开关报错**：`set_autostart` 误用 `sys.os.path`（sys 无 os 属性）
  导致勾选时报 "module 'sys' has no attribute 'os'"，改用局部导入的 os
- **网卡下拉列出全部 Up 状态网卡的 IPv4**：TUN/VPN 虚拟口不再从列表硬剔除
  （用户可显式选择），仅排序沉底；自动选择（best_lan_ip）仍优先非 TUN 物理口，
  mDNS 未指定接口时也只广播物理口
- **TLS 证书跟随网卡重签**：证书 SAN 原先一次生成永不更新，换网络后新 IP 不在
  SAN 内导致浏览器客户端 TLS 主机名校验失败；现 `ensure_tls_cert` 在 SAN 未覆盖
  当前全部网卡 IP 时自动重签，服务器热加载新证书链（已有连接不断）
- **网卡变化自动跟随**：低频轮询 watcher（5 秒），网卡集合或选中 IP 变化时自动
  跟随（证书重签/mDNS 重注册/下拉与二维码刷新统一走一条切网路径）

## 2026-08-21 — Lite 界面尺寸体系重构：分辨率自动挡位

- **分辨率自动定挡**：新增 `RES_GEARS` 门槛表，按屏幕等效高度（宽度按 16:9 折算）自动选挡
  （768→85%、900→95%、1080→100% 基准、1152→110%、1440→125%、1440+带鱼屏→145%、4K→175%），
  启动即定挡，换显示器免调；托盘「缩放比例」菜单新增「自动（按分辨率）」项，手动百分比可覆盖并记忆
- **一套尺寸表驱动全部组件**：新增 `make_sizes(zoom)`，每个挡位对应一组确定 px 值
  （字号/按钮高/下拉行高/标题栏/间距/窗口基准），所有组件只从表取值，杜绝混排
- **像素字号替代 pt 字号**：全部字体改 Tk 负数字号（px），`tk scaling` 固定 1 不再参与缩放，
  命名 Font 对象全局共享，换挡改一处字号全界面自动重排，删除 `_refresh_fonts` 全树遍历补丁
- **同行控件严格等高**：增益输入框改为固定高度外壳 Frame（与下拉框同手法），
  高度对齐按钮实测需求高，消除 Entry 边框固有差；前后增益行字号统一为同一像素值，
  四个加减按钮统一像素字体（原 Arial 与像素字混排、两行字号不一致）
- **清理**：删除空转的 `_poll_dpi` 轮询、从未被调用的 `_apply_pixel_font` 死代码、
  pystray 失败时的 BeautifiedTray 兜底托盘（单一实现路径；CI 同步补装 pystray/pyaudio 并显式打包）

## 2026-08-20 — 切换到 Python 3.12（PyAudio 暂无 3.13 wheel，自包含）

- **Python 3.13 → 3.12（自包含）**：内嵌 Python 由 3.13.7 切换到 3.12.11（`bootstrap_python312.sh/.ps1` + `py312` 启动器、`packages/python312*` / `.py312-src`、`packages/cpython@v3.12.11`），CI 基线同步 `python:3.12-bullseye` / `setup-python 3.12`，`pack_deb.sh` / `pack_appimage.sh` / `build_win.ps1` 路径同步为 `python312`，原因：目前 Python 3.13 暂无 PyAudio 预编译 wheel，切到 3.12 保证 `pip install PyAudio` 有 wheel 可用
- **影响范围**：CI 与内置运行时统一为 3.12，无内置回退分支

## 2026-08-15 — 升级至 Python 3.13 + ONNX Runtime 1.22

- **Python 3.8 → 3.13**：内嵌 Python 迁移至 3.13.7（`bootstrap_python313.sh/.ps1` + `py313` 启动器，`packages/python313*` / `.py313-src`），`--with-ensurepip` 编译，Win7 支持终止（最后 Win7 版为 v2026.08.14.1643）
- **ONNX Runtime 1.11.1 → 1.22.0**：Linux/Windows 捆绑包同步升级至 1.22.0（`packages/onnxruntime-*-1.22.0`），`setup.py` / `aimic.py` / 打包脚本路径同步更新，CI 容器 `python:3.13-bullseye` 验证
- **PySide6 解锁**：`PySide6==6.1.3` 锁死移除，改为 `PySide6>=6.8`（Python 3.13 需新版 Qt），cryptography 锁 42.0.8（Win7/py<3.9）同步移除

## 2026-08-14 — 修复 EQ 预设/插槽按钮点击无反应（PySide6 6.1.3 传参 bug）

- **现象**：均衡器面板的预设按钮与插槽按钮点击无任何效果，启动日志反复出现
  `TypeError: <lambda>() missing 1 required positional argument: '_'`
- **根因**：PySide6 6.1.3 的 `clicked` 信号对「带默认参数的 lambda」
  （`lambda _, slot=i:`）传参有 bug——按 lambda 参数个数（0 个额外）传参，导致
  位置参数 `_` 缺失，回调抛 TypeError 不执行。此前 `lambda _:`（无默认参数）
  恰好能接住信号发出的 1 个参数，故单参数 lambda 正常，仅 EQ 这两处带默认
  参数的 lambda 出问题，线上未暴露
- **修复**：EQ 两处连接改用可变参数吸收信号参数
  `lambda *a, slot=i:` / `lambda *a, n=name, g=gains:`，消除位置参数不匹配
- 最小复现已确认：修复前点击报 TypeError，修复后正常

## 2026-08-14 — deb 捆绑内嵌 Python + PySide6 瘦身，跨发行版兼容

- **deb 捆绑内嵌 Python 3.8**：`pack_deb.sh` 现把 `packages/python38`
  （含 PySide6 6.1.3 与 zeroconf/aiohttp/cryptography/opuslib 全部依赖）整个
  拷进 `/opt/purevox/python38`，启动脚本改用内嵌 python（`PYTHONHOME` +
  `LD_LIBRARY_PATH`），与系统 Python 及发行版 python 包名（AOSC 的
  `pyside6`/Debian 的 `python3-pyside6` 命名各异，且 Debian 无 PySide6 apt
  包）彻底隔离，与 AppImage 同一实现路径。`Depends` 现只留原生 C 运行库
  `pipewire`、`libasound2`，不再声明任何 Python 依赖，跨发行版可安装即用
- **libcrypt.so.2 兼容软链**：内嵌 python 3.8.20 由 AOSC GCC 15 编译、链接
  `libcrypt.so.2`，较新发行版（如 Debian 13）只有 `libcrypt.so.1`
  （libxcrypt，ABI 兼容），打包时补软链使其可加载
- **PySide6 瘦身（560M→115M，deb 153M→83M）**：应用只用 QtWidgets/QtCore/
  QtGui，砍掉 `Qt/qml`(337M)、examples、3D/Charts/Sql/Svg/Quick 等模块与
  非必要 plugins，只留依赖闭包。依赖闭包实测确认：`libpyside6.abi3.so` 硬
  依赖 `libQt6Qml`（与 Windows 同约束勿删）；`libqxcb` 需 `libQt6OpenGL`，
  缺失会解析到系统 Qt 版本冲突。瘦身逻辑提取为 `scripts/slim_pyside6.sh`，
  `pack_deb.sh` / `pack_appimage.sh` 共用（单一实现路径）

## 2026-08-14 — deb 打包依赖增强跨发行版兼容（Debian）

- **Python 依赖移出硬 Depends，改入非阻塞 Recommends**：此前 deb 的
  `Depends` 按 AOSC 包名写死 `python-3 (>= 3.13)`、`pyside6`、`zeroconf`、
  `aiohttp`、`cryptography`、`opus`、`opuslib`，在包名不同的 Debian 上安装
  即报 "but it is not installable" 直接失败。现仅把原生 C 运行库
  （`pipewire`、`libasound2`）留在 Depends；Python 依赖全部改到 Recommends，
  并用「AOSC 名 | Debian 名」备选（如 `pyside6 | python3-pyside6`），
  哪个发行版能解析哪个就尽力装上，解析不到也只是跳过、不再阻塞安装
- 说明：App 运行仍需要这些 Python 包，安装后缺失时请用系统包管理器或
  `pip install --user pyside6 zeroconf aiohttp cryptography opuslib` 补装

## 2026-08-13 — 修正 Linux 设备枚举认知：按声卡枚举，数字/模拟双麦克风皆真实

- **修正设备枚举认知：数字/模拟双麦克风皆真实**：数字麦（Digital Microphone/Mic1）
  与模拟麦（Stereo Microphone/Mic2）是**同一块声卡（如 sof-hda-dsp）的两个接口，
  各对应一个真实物理麦克风**，二者都应列为输入；旧版按 `api.alsa.path` 无 `,dev`
  把 Mic2 误判排除，导致 PipeWire 少列一个麦（与 ALSA 接口宽松枚举不一致）
- **设备枚举按声卡组织**：Linux 一个声卡有多个接口（Mic1 数字麦 / Mic2 模拟麦 /
  扬声器 / HDMI 等），各接口对应真实设备，需在系统声卡设置（pavucontrol /
  alsamixer / UCM）里激活对应接口，接口即对应到该设备的声音
- PipeWire 与 ALSA 接口的物理麦克风枚举逻辑对齐（都宽松列出双物理麦）

## 2026-08-13 — Linux 新增本地接口 ALSA（原生备选）

- **音频接口下拉框改为「本地接口 PipeWire（默认）」+「本地接口 ALSA」+「网络(API)」三项**：
  Linux 本地输入/输出默认仍为原生 PipeWire，新增原生 ALSA 备选后端（`alsa_client.c` →
  `libpvalsa.so`，ctypes 绑定 `pvalsa.py`），供无 PipeWire 的极简/纯 ALSA 系统或
  需要绕过 PipeWire 的场景使用
- **ALSA 桥 AlsaBridge**：仿 PwBridge，单个 I/O 线程用 poll() 驱动 capture/playback/
  monitor/far 四个 PCM，数据路径只碰无锁 SPSC 环形缓冲；F32 单声道 48000Hz 经
  `plughw:C,D` 插件层转换（speex/samplerate 重采样 + 声道/格式转换），模型永远拿 48k
  单声道。`setup.py` Linux 构建新增 `_build_pvalsa_linux()`（pkg-config alsa）
- **监听与 AEC**：监听 = 降噪音频多输出一份到选定 playback 设备（mon PCM）；AEC far =
  从用户选定的 capture 设备读扬声器输出（far PCM，须选可捕获输出的设备名）
- **设备枚举**：`get_device_names` 在 api_type==ALSA 时用 `arecord -l` / `aplay -l`
  解析出 `plughw:C,D` 名（下拉显示名、userData 存 plughw 名），PipeWire 仍走 pw-dump
- **配置键复用**：ALSA 走现有 `input_device_alsa` 等占位键；Linux 默认 api_type 由
  Pulse(15) 改为 PipeWire(98)，老配置自动回退到新默认
- **虚拟麦克风统一中转（一个设备兼顾两种接口）**：`purevox_out` 是唯一虚拟麦克风
  中转，PipeWire 与 ALSA 两路降噪输出都汇入它，不引入 snd-aloop、不建第二套虚拟
  麦克风。**ALSA 接口是混合实现**：输入走 ALSA，输出到虚拟麦克风**必须用 PipeWire
  原生流（PwBridge）显式写 `purevox_out`**——实测 `pcm.pulse`/默认 sink 中转虽然
  `purevox_out.monitor` 有信号，但 `purevox_mic`（remap-source 真源，供 OBS 等只列
  真源软件）静音；只有 PipeWire 原生 `pw_stream` 写 `purevox_out` 才能驱动
  `purevox_mic` 取数。UI 语义与 PipeWire 模式看齐（输入/输出/监听三个下拉）。
  虚拟声卡面板按当前接口显示引导文案
- **ALSA 输入必须用 `pulse:<物理麦克风>` 显式指定（关键）**：`pcm.pulse` 读默认
  source，但 `purevox_mic` 抢占默认 source 且 `pactl set-default-source` 改不回
  （exit=0 无效），导致 pcm.pulse 输入**回读 PureVox 自己输出**（实测读到回读正弦
  rms=0.27）。ALSA 输入下拉改用 `pulse:<node.name>`（pw-dump 枚举物理麦克风，排除
  `purevox_mic`/`*.monitor`）显式指定源绕开默认；无 PipeWire 的纯 ALSA 系统用
  `plughw:C,D` 直连。本机板载 sof-hda 声卡的数字麦（Mic1）与模拟麦（Mic2）是
  两个**真实物理麦克风**（声卡的两个接口，需在系统声卡设置里激活对应接口），
  Mic2 实测 `pulse:Mic2` 可打开但拾音极弱（模拟麦灵敏度/增益特性，非假设备）
- **设备枚举**：ALSA 模式输入/输出下拉前置物理麦克风（`pulse:<source>`）与虚拟麦克风
  （`purevox_out`），并列出 `plughw:C,D` 物理设备；注意大量 HDMI 为**未连接假设备**
  （本机仅 Headphones 输出 + Mic2 输入可用），真正可用端点通常只有物理耳机/板载麦
- **修复 ALSA 停止卡死**：AlsaBridge 关闭时对 capture 流（in/far）用 `snd_pcm_drain`
  会**无限阻塞**（实测 pcm.pulse 采集 stop 时 UI 卡死无退出）；改用 `snd_pcm_drop`
  （立即丢弃），playback（out/mon）保留 drain 安全等待

## 2026-08-13 — Windows 新增本地接口 MME + 设备配置按接口隔离

- **音频接口下拉框改为「本地接口 WASAPI（默认）」+「本地接口 MME」+「网络(API)」三项**：
  Windows 设备枚举与打开流都按所选接口过滤（`get_device_id` / `get_device_names` /
  48k 检测走 `get_host_api_indices` 分级匹配 host API），不再只有一个本地接口
- **WASAPI 仍为默认**：默认值、老配置（api_type=13）均不受影响；MME（PortAudio
  paMME=2）为旧版低延迟备选，仅当 WASAPI 不可用时使用
- **设备配置按接口隔离，显式写全**：通用键 `input_device` / `output_device` /
  `monitor_device` / `aec_far_sink` 废弃，改为 `<方向>_device_<接口后缀>` 与
  `aec_far_sink_<接口后缀>`（如 `input_device_wasapi` / `input_device_mme` /
  `input_device_pulse`）。`config_manager.py` 的 `ConfigDefaults` 与 `_KEY_ORDER`
  把全部接口（WASAPI/MME/PulseAudio/ALSA/DirectSound/ASIO/Core Audio/OSS/JACK/
  Sndio）的键显式写全，不做动态生成，阅读直观
- **monitor 与 AEC far 分开维护**：监听设备存 `monitor_device_<接口>`，AEC 回声
  参考 sink 存 `aec_far_sink_<接口>`（Linux 手动选物理扬声器），同一下拉框按模式
  写不同键，互不覆盖
- **默认不置空、不特选 CABLE Input**：配置设备值为空或不在枚举列表时，强制回退
  枚举列表**第一个**并写回配置（用户自行选择 VB 用法）；删除了 Windows 输出默认
  CABLE Input 与 UI 自动偏好
- **强配置，不做旧配置迁移**：`ConfigManager.load_config` 只保留已知键，
  旧 `WASAPI_*` / 通用设备键等未知键一律丢弃回退默认；网络模式输出/监听/AEC far
  仍用平台默认接口的键
- **host API 匹配分级化**：`get_host_api_indices` 改为分级匹配——先按配置的 API 名
  （选 MME 就只列 MME，不再混入 WASAPI），匹配不到再回退平台默认，最后全枚举兜底

## 2026-08-12 — 移除编译期 `-mavx2`，全 CPU 兼容

- **`setup.py` 编 `aimic.dll`/`libaimic.so` 去掉 `-mavx2`**：一律用 `-O2`
  默认基线（x86-64 的 SSE2）。原因：`-mavx2` 让 gcc 对整个 aimic.c+pffft+
  libsamplerate 无差别生成 AVX 指令且不做运行时检测，在无 AVX 的老 CPU
  （酷睿4代以前）上，pffft FFT 热身即 `0xc000001d` 非法指令崩溃。
  实测：带 `-mavx2` 时 DLL 含 3888 条 AVX 指令、推理 4.010ms/帧、老 CPU
  崩溃；去掉后 0 条、5.629ms/帧、全 CPU 兼容（远低于 20.8ms 实时预算）。
- **模型推理性能不受影响**：onnxruntime 的 MLAS 内核在运行时按 CPUID
  dispatch（SSE2→AVX→AVX2 自动选最优），与本库编译参数无关。
- **「推理后端」日志改为纯报告**：`cpu_supports_avx()` 探测 CPU 能力并打印
  AVX/SSE，仅反映 MLAS 会跑哪档内核，不参与行为决策；NPU 分支保留为**死代码
  示例**（捆绑 onnxruntime 1.11.1 为纯 CPU 构建，DirectML/OpenVINO EP 必然
  失败），作为日后加入 NPU 执行提供程序的参考路径，见 `aimic.c`
  `onnx_apply_backend` 的 TODO(NPU) 注释。

## 2026-08-12 — 48kHz 启动检测补诊断日志 + 修复中文设备名乱码

- **48k 检测失败时输出诊断日志**：`_try_open_48k` 失败打 `[warn]`，包含设备索引/
  名字/默认采样率/输入输出通道数/宿主 API/异常原因；每次启动输出一行汇总
  （`[48k检测] 输入=OK 输出=FAIL ...`）；弹框阻止启动时打 `[err]` 列出设备名。
  用于区分「设备真不支持 48k」与「设备被占用/独占」等资源性问题。
- **修复中文设备名乱码**：PortAudio 返回 UTF-8 设备名，PyAudio 在中文系统
  （locale=cp936/GBK）先按 GBK 解码、不抛异常就不退回 UTF-8，导致部分中文名
  （如「线路输入」）变成「绾胯矾杈撳叆」式乱码，且同批设备有乱有正常。
  新增 `device_api.fix_device_name()`：对乱码串按 GBK 重编码再按 UTF-8 解码还原，
  正常名字不受影响。`get_device_names` / `get_device_id` 与 48k 诊断日志统一走它。

## 2026-08-12 — 移除 VB-CABLE 检测的 PnP 退化回退

- `dialog_vbcable_check.py` 虚拟声卡检测不再用 PowerShell `Get-PnpDevice` 兜底：
  只走 PyAudio 枚举双端点（CABLE Input 有输出通道 + CABLE Output 有输入通道，
  限时 5s）。原退化路径会误判「驱动已装但被禁用 / 半卸载残留」的设备为已安装，
  且 Get-PnpDevice 需额外拉起 PowerShell（~1s、无窗口）。
- 检测单一实现路径：不做任何驱动层/PnP 兜底，已安装判定唯一依据是
  PortAudio 能枚举到可用的 CABLE 双端点

## 2026-08-11 — 推理后端改为自动选择（NPU → AVX → SSE）

- **移除「推理后端」下拉框**：后端不再需要手动选择，创建模型时自动按
  NPU → AVX → SSE 顺序选取：优先尝试 NPU 执行提供程序（Linux OpenVINO /
  Windows DirectML），不可用则回退 CPU——CPU 支持 AVX 用 AVX，不支持则 SSE
  （ORT 内核本就按 CPUID 自动选择最佳指令集）
- **实际生效后端仍可见**：启动日志打印 `[启动] 推理后端: ...`，若 NPU 不可用会
  注明原因；捆绑的 onnxruntime 1.11.1 是纯 CPU 构建（已移除 SSE 限制配置项、
  CPU EP 不读任何会话配置，实测确认），故当前实际生效为 AVX
- 移除 `inference_backend` 配置键（不再需要）

## 2026-08-11 — 降低 PipeWire 状态日志频率

- `[PipeWire] 处理 N 帧 (rms=...)` 状态日志由每 100 帧（约 2 秒）改为每 1000 帧
  （约 21 秒）打印一次，减少刷屏

## 2026-08-11 — 窗口标题版本号随 tag 打包

- **Linux 不再显示「开发版」**：deb / rpm / AppImage 打包脚本按 tag
  （`GITHUB_REF_NAME`）生成 `_build_version.py`，窗口标题与包版本/文件名同源；
  本地手动打包回退当前时间
- **Windows 标题日期取 tag**：`build_win.ps1` 生成版本戳时优先用
  `GITHUB_REF_NAME`（v<yyyy.MM.dd.HHmm> → yyyy-MM-dd-HHmm），不再用构建机本地时间

## 2026-08-11 — 移除窗口隐藏时的频谱调试日志刷屏

- `_feed_visualizer` 中遗留的 `[调试] _feed: ...` dev 日志在窗口最小化到托盘时每帧
  打印一条，已删除该调试分支（频谱仅在可见时更新，行为不变）

## 2026-08-11 — Windows 设备枚举对齐 WASAPI

- **Windows 设备枚举只留 WASAPI host API**：`get_device_names` / `get_device_id`
  不再枚举全部 host API，改为先经 `device_api.get_host_api_indices` 过滤
  `dev['hostApi']`，避免混入 DirectSound / MME / WDM-KS 等重复或无关端点
- **修 host API 名字匹配**：`device_api.get_host_api_indices` 原用精确相等
  （`info['name'] in names`），但 PyAudio 的 host API 名带厂商前缀
  （WASAPI 实为 `Windows WASAPI`），匹配不到误触「全枚举兜底」→ 等于没过滤。
  改为大小写不敏感的子串匹配；名不见时仍回退全枚举（虚拟 sink 等跨 API 设备）
- Linux 不受影响：仍是原生 PipeWire pw-dump 节点枚举

## 2026-08-11 — 用户手册与更新日志内化到「关于」对话框

- **删除两个独立文档文件**：`用户手册.html` 与 `CHANGELOG.md` 删除，内容全部内嵌
  `dialog_about.py`；菜单改为单一「关于」入口，打开即整页标签：软件介绍 /
  Windows 使用说明 / Linux 使用说明 / 更新日志 / 许可证
- **更新日志改内嵌维护**：从本版起不再有独立 CHANGELOG.md 文件，发版时直接在本
  `CHANGELOG_TEXT` 顶部追加（见 AGENTS.md）
- **新增 Linux 使用说明页**：原生 PipeWire、虚拟麦克风双出口、AEC / 远程麦克风、
  常见问题；Windows 页为原手册内容
- **Windows 说明按现状修正**：音频接口只保留 WASAPI（不再提 MME）、删除后增益
  （post_gain 已移除）；新增 48kHz 强制检测说明——启动弹框提示设备不支持 48kHz 时，
  去 Windows 声音控制面板把设备默认格式设为 48000 Hz 后重试
- **去掉 emoji**：手册等用户可见文本不再使用 emoji（不引入 awesome 风格）
- 引用处同步：README（中/英）链接、AGENTS.md、build_win.ps1（不再拷贝 CHANGELOG.md）

## 2026-08-11 — VB-CABLE 检测面板重设计

- **检测只弹未安装**：`_check_vbcable` 改为先检测，VB-CABLE 已安装则无事发生，
  未安装才弹面板；检测开关（`vbcable_check_enabled`）默认开启，取消勾选即跳过
- **面板统一结构**（`dialog_vbcable_check.py`）：已安装/未安装共用同一布局——
  状态灯 + 双端点说明（CABLE Input 接收 PureVox 输出 / CABLE Output 作虚拟麦克风，
  均 48kHz，含数据流向示意）+ 驱动卡片；未安装时卡片额外显示红色安装提示
- **VB 控制面板按钮移入面板**：菜单栏删除「VB设置」入口（及 `open_vb_panel`），
  面板驱动卡片内新增「打开控制面板」按钮（未安装时置灰）
- **面板自动刷新状态**：2s 定时器重查安装状态，装好驱动后指示灯无需重开即变绿
- 面板新增公开的 `vbcable_installed()` 供启动检测复用

## 2026-08-10 — 开源重构定稿（08-07 起的最终状态，中间过程已压缩）

> 08-07 开始的 CI/构建/兼容/音频架构重构均已定稿；被后续推翻的中间实现
> （pybind11 绑定、旧 onnxruntime 版本、`module-null-sink` 虚拟麦克风、VB-CABLE
> 自动安装等）不再重复记录，这里只保留最终生效的状态。

### 版本号与发版（2026-08-10 追加）

- **版本号由构建时间驱动改为 tag 驱动**：deb/rpm 包内 `Version` 与 `setup.py` version
  不再写死 `1.0.x`。tag 触发 CI 时直接从 tag 名推导 `yyyy.MM.dd.HHmm`
  （`v2026.08.10.1517` → `2026.08.10.1517`），文件名字段转 `yyyy-MM-dd-HHmm`（含
  `pack_appimage.sh` 与 APK 改名）；
  各 job 并发跑不再各自 `date`，杜绝产物文件名/版本号互相漂移不一致。本地/手动跑
  （无 `GITHUB_REF_NAME` tag）才回退到构建时刻。
- **push tag 自动发 release**：`release.yml` 新增 `release` job——推送 tag
  `v<yyyy.MM.dd.HHmm>`（如 `v2026.08.10.1517`）时，三个构建 job 针对该 ref 重跑，
  随后自动 `gh release create` 并把全部产物 attach 到 Release（Linux deb/rpm/AppImage、
  Windows 目录重打成 zip、Android APK）。tag 命名与包内版本号对齐，二者来自同一来源。
- **CI 触发收紧**：只有 push tag（`v*`）才跑 CI，分支 push 不再触发；
  需验证分支用 `workflow_dispatch` 手动跑。日常快速提交零 CI 成本。

### 构建与运行时

- **内嵌 Python 3.8（独立于系统环境）**：Linux `bootstrap_python38.sh`（git 子模块
  `packages/cpython`@v3.8.20 out-of-tree 编译，自带 `py38` 启动器）；Windows
  `bootstrap_python38.ps1`（NuGet 预编译包 → `packages\python38w\`）
- **纯 C 共享库 + ctypes 取代 pybind11**：`aimic.c` → `libaimic.so`/`aimic.dll`
  （pffft + libsamplerate + ONNX Runtime C API）、`pipewire_client.c` → `libpvpipe.so`
  （无锁 SPSC + pthread）；绑定层 `aimic.py`/`pvpipe.py`。项目不再含任何 C++/pybind11
- **ONNX Runtime 统一捆绑 1.11.1**（win/linux 预编译 SDK，不 pip 安装）；setup.py
  保留 `ORT_INCLUDE_DIR`/`ORT_LIB_DIR` 覆盖；运行时 `LD_LIBRARY_PATH` 注入
- **Windows `aimic.dll` mingw 构建打通**：`windows.h` 后按平台 `#undef far/near`
  （win 头 16 位空宏吞 `far`/`near` 参数名导致编译失败）；setup.py Windows 分支链接
  捆绑 ORT import lib；`.so`/`.dll` 用固定名定位（不再用 `sysconfig.EXT_SUFFIX`）
- 打包：`build_win.ps1`（PyInstaller one-folder → 产物目录 `dist/PureVox/`）、
  `pack_deb.sh`、`pack_rpm.sh`、`pack_appimage.sh`
- **CI 精简为单一 `.github/workflows/release.yml`**：linux（ubuntu → deb + AppImage
  best-effort / fedora → rpm / python3.8 最低运行时冒烟）、windows（mingw 编
  `aimic.dll` + 打包上传）、android（debug APK，JDK17+SDK34+NDK27）；actions 全部
  升级 node24
- **产物命名统一**：`PureVox-<平台>-<架构>-<yyyy-MM-dd-HHmm>-<release|debug>.<ext>`
  （`pack_rpm.sh` 产物名与 CI 上传 glob 由同一 `$PKG_FILE` 变量驱动，杜绝上次 glob
  不匹配导致 fedora job 挂）
- **AppImage 修复**：ubuntu job 装 `libssl-dev`（内嵌 CPython 编译 ssl 模块否则 pip
  无网络）与 `file`；desktop/icon 副本补 `$APPDIR` 根目录；`PyAudio` 移入
  `requirements-win.txt`（Linux 原生 PipeWire 不需要，此前 Linux 编译缺
  `portaudio.h` 让 AppImage 打包静默失败）
- **CI 踩坑沉淀**（多轮 CI/容器实测累积，勿白踩）：
  - 容器 checkout：先在 checkout 前装系统依赖（含 `git`），REST API 下载不支持
    submodules；仅 AppImage job 拉 `packages/cpython` 子模块（fedora/python3.8 不拉）
  - AppImage：appimagetool 容器无 FUSE → 用 `--appimage-extract-and-run`；`.desktop`
    须同时放一份到 AppDir 根目录；图标由 `audio_icon_base.png` 直接生成
    256/512 png（勿走 ico 转换，避免 `Icon not found`）
  - `pipewire_client.c`：勿 `#include <spa/param/audio/raw-utils.h>`（老 spa /
    bullseye 没有该头）；`PW_KEY_TARGET_OBJECT` 是 0.3.64 才引入，老版本需回退
    `node.target`
  - `pack_deb.sh` 末尾 `dpkg-deb --info | head` 触发 SIGPIPE(141) 让 `sh -e` 退出
    → 补 `|| true`
  - Ubuntu 容器 pip 装 pillow 遇到匹配版本时用 `--break-system-packages` 兜底
    （`||` 回退普通安装），不再 `pip install --upgrade`；`python3-setuptools`
    由 apt/dnf 装机（sysdeps 里）供 setup.py
  - Android：JNI `CMakeLists.txt` 版权头必须用 `#` 注释（CMake 不认 `//`，否则
    Parse error）；`gradlew` 仓库中无执行位，构建前先 `chmod +x`
  - Windows：pwsh 没有 `\` 行继续符（compileall 多行命令被拆行执行）→ 写单行；
    `aimic.dll` 编译统一走 `build_win.ps1` 一条路（独立 Build 步骤已删，不再维护
    `-SkipC` 双路径）

### Windows 7 兼容（实测结论，勿回退）

- **PySide6 锁 `6.1.3`**（最后一个支持 Win7；Qt 6.2+ 官方仅 Win10+，6.6.x import
  即报 `DLL load failed`）；四件套 wheel 含 `abi3`
- 打包时补 Win7 缺失 DLL：两个 **API-Set 转发 stub**（仓库固化 x64，导出符号转发到
  KERNEL32，mingw 可复现）+ **MSVC 运行库**（MSVCP140/VCRUNTIME140 等），Win7 无需
  单独装 VC++ redist
- **瘦身禁删 `Qt6Qml.dll`/`Qt6Quick.dll`**：`pyside6.abi3.dll`（所有 Qt*.pyd 的链接
  目标）硬依赖 `Qt6Qml.dll`，删除则 Win7 启动即报模块缺失；只允许删 import 闭包外的
  `Qt6Pdf.dll`/`Qt6DataVisualization.dll`
- **深色主题判定一律用调色板亮度**（`theme_colors.is_dark_current()`），禁
  `styleHints().colorScheme()`（Qt 6.5+ API，6.1.3 没有）

### Linux 音频架构（原生 PipeWire）

- Linux 输入/输出/设备枚举/AEC 全部原生 PipeWire，移除 Linux 端 PyAudio 路径
  （TSE 参考音频播放除外）；格式一律 **F32 单声道 48kHz**
- **AEC far-end**：独立 `PureVox-far` 流（`stream.capture.sink` tap 扬声器 sink，
  恒 48k 单声道免重采样，会话内创建/销毁）
- **虚拟麦克风定稿（单一生产者 + 双出口，全健康）**：
  生产者 = 单声道 null-sink `purevox_out`（`pw-cli create-node`，唯一写入口）；
  出口 1 = 内置 monitor `purevox_out.monitor`（宽口径，普通软件直接选用）；
  出口 2 = `module-remap-source` 重映射真源 `purevox_mic`（`media.class=Audio/Source`、
  无 monitor_of，供 OBS 等"只列真源"软件）。生命周期 `virtual_mic_ready()` /
  `ensure_virtual_mic()` / `remove_virtual_mic()` 全幂等；**启动不再自动创建**，
  菜单「虚拟声卡」→ `dialog_virtual_mic_linux.py` 手动创建/清理
- **本地输出流低延迟**（唯一优化过的延迟路径，网络模式不受益）：
  `create_stream` 显式 `SPA_PARAM_Buffers`(4096B=1024 样本) → 输出流缓冲 12288→1024
  样本（256ms→21ms）；`RING_CAPACITY` 收窄到 4 hop(4096/85ms) 封顶，输入环稳态 ~0
- 设备枚举修正 USB 麦克风误判（`device.bus=usb` 直接放行）；VU 电平显示
  降噪输出峰值
- **禁用/踩坑（违反即弄坏系统托盘/协议）**：禁 `module-null-sink
  media.class=Audio/Source/Virtual`（弄坏 pipewire-pulse）、禁 `pw-loopback`（旧架构，
  仅防御性 pkill）、禁重启 pipewire-pulse 修托盘（KDE 不自动重连）、禁 PortAudio
  直开 null-sink（ALSA 插件堆崩溃）

### Windows 虚拟声卡

- **VB-CABLE 改为手动安装**：`dialog_vbcable_check.py` 纯检测弹框（不再自动安装，
  给官方驱动包下载链接 + B站教程，状态指示灯）；复选框「检测虚拟麦克风」默认开启，
  取消即跳过不再提示（config `vbcable_check_enabled`）

### UI / 配置 / 规约

- 修 `SegmentedControl` 模式按钮在 PySide6 6.6 点击无响应（参数推断错误）→
  `lambda *_args, v=val`；系统声音面板改 `subprocess.Popen` 异步打开（不再超时杀进程）；
  cryptography 锁 `==42.0.8`（py<3.9 线，46.x 起拆出 `cryptography_rust.dll` Win7 缺件崩）；
  `open_sound_panel`/`list_sources` 等平台修正
- AGENTS.md 新增工程约定：C 源码禁止中文（第 12 条）、弹框文件统一 `dialog_` 前缀
  （第 13 条）；全部源码头注释更新 GPL-3.0 + 模型声明（`MODEL-LICENSE.md`）

---

## 2026-07-31 — TSE模型 tse15_stream_ep_0673

- 更新 TSE 模型文件，替换为新版本 tse15_stream_ep_0673.onnx

---

## 2026-07-30 — TSE模型 tse15 & 网络串流更新

- 更新 TSE 模型文件，替换为新版本 tse15_stream_ep_0350.onnx
- 更新安卓APK和web串流UI和效果
- 新增压缩效果(测试)

---

## 2026-07-29 — 降噪模型 purevox9 & TSEv15 & 网络串流更新

- 更新 purevox9 模型文件，替换为新版本 v9_fft2048_band256_epoch_261.onnx
- 更新 TSE 模型文件，替换为新版本 tse15_stream_ep_0081.onnx(测试版)

---
## 2026-07-21 — 降噪模型 purevox9

- 重写v9降噪模型，测试版初版发布，仅供测试不代表最终品质，修复purevox9低频噪音问题，提升任意频段的说话时噪音消除能力
- “菜单栏”新增手动主题设置，默认为“系统”，可手动指定为“白天/黑夜”

- **WASAPI 全双工采样率修复**：非 48K 设备回退半双工 + libsamplerate 重采样，修复异常声音（详见 docs/wasapi-full-duplex-samplerate.md）

---

## 2026-07-20 — 录音倒数 & AEC v9

- 录音按钮点击后显示 3→2→1 倒数，倒数结束才开始录音
- 录音/播放按钮加宽，视觉更平衡
- 播放中按钮变"停止"，可点击中断
- 录音完成后自动加载最新参考音频，无需手动切模式
- 前增益数值标签留更多空间，避免挤在一起
- VU 电平表刻度字号缩小，右侧留白增加
- 优化内存占用，模型按需加载
- AEC 升级为独立模型 v9-ep483，不再依赖降噪模块，回声消除效果大幅提升，能更好去除音响声、保留人声
- 降噪升级至 v9，去除噪音能力略有减弱，后续可考虑在 AEC 后串联降噪进一步优化

---

## 2026-07-19 — 架构精简 & 按需加载

- 删除 post_gain（后增益），处理链路统一为 `前增益(AGC替代) → EQ → clip → [AEC] → 降噪 → [TSE] → clip → VAD → 输出`
- 四种模式：直通 / 降噪 / AEC / TSE（录音改为 TSE 子功能）
- 模型按需加载/释放：降噪、TSE、AEC 仅在对应模式下加载，切换时自动释放
- EQ 按需：全 0 时跳过处理
- 配置白名单：移除 model_name、aec_enabled、DirectSound 设备等废弃项
- UI 清理：删除 qtawesome 依赖，全局样式精简
- 关于对话框：使用说明更新为四模式，Qt 页改为 LGPL v3 标准免责声明

---

## 2026-07-17 — MME 驱动修复

- MME 模式下消除机器音，自动适配设备真实采样率
- 启动日志显示设备索引，方便排查

---

## 2026-07-16 — V8 模型 + 频谱图 & 均衡器

- V8 降噪模型
- 频谱直方图：128段 Mel 实时降噪前后对比
- 61段均衡器：7组预设，可视化曲线拖拽
- 修复模式切换闪退、EQ 拖拽崩溃

---

## 2026-07-03 — V6 模型 + VU 优化

- V6 降噪模型：人声保留提升，突发噪音识别减弱
- VU 电平表：Peak 峰值，-20/-9dB 分界，16ms 刷新
- 托盘暂停 UI 定时器，窗口恢复自动重启
- HTML/Android 客户端 VU 同步更新

---

## 2026-07-02 — 网络串流 + 高质量重采样

- 网页端远程麦克风（HTTPS+WSS）
- Android App 局域网连接，Opus 编解码
- mDNS 自动发现，一键热点
- 防火墙自动放行
- 高质量重采样（44.1k/48k 等）
- 设备热插拔自动刷新
- 修复网络音频变调、半双工卡顿、端口冲突等

---

## 2026-06-28 — 局域网远程麦克风

- 手机 App / 浏览器作为远程麦克风
- 局域网自动发现，断网重连

---

## 2026-06-27 — VU 电平表 + 文件处理

- VU 实时电平表
- 文件降噪导出

---

## 2026-06-13 — PySide6 重构

- 直通 / 降噪 / AEC / TSE / 录音 五种模式
- tkinter → Qt (PySide6)
- AEC 回声消除，AGC 自动增益
- 睡眠唤醒自动重连

---

## 2025-10-09 — Windows 原生音频

- WASAPI/MME，即插即用

---

## 2025-08-13 — 正式版

- 降噪效果提升，延迟更低

---

## 2025-07-23 — 2.0 测试版

- 48kHz 采样率，降噪提升

---

## 2025-04-19

- 增益优化，静默启动，多 DPI 适配

## 2025-04-08

- 设备按 ID 识别，不再因顺序变化选错

## 2025-04-02

- 开机自启

## 2025-03-29

- 增益 -20~+30dB，配置自动保存
