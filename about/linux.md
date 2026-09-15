# PureVox 使用说明（Linux）

## 安装与启动

Linux 版音频采集/输出/虚拟麦克风走 **pipewire-pulse 兼容层**（自研 ctypes 绑定直调系统 **libpulse**），
不再编译任何扩展。请安装并运行：

- **pipewire** + **pipewire-pulse**（提供 PulseAudio 协议服务）
- **libpulse**（客户端库，`libpulse.so.0`）
- **libopus**（可选，网络推流解码用；缺失时仅该功能降级）

```bash
# Debian / Ubuntu
sudo apt install pipewire pipewire-pulse libpulse0 libopus0
# AOSC（按发行版补 libpulse / pipewire-pulse / libopus）
sudo oma install -y python3 pipewire
```

确认服务在跑：`systemctl --user status pipewire-pulse`（是 **pipewire-pulse**，不是 pipewire）。

运行方式（任选其一）：

1. 安装包：按发布页下载 deb（Debian/Ubuntu 系）、rpm（Fedora 系）或 AppImage，
   安装后从桌面/应用菜单启动，或命令行执行 purevox ；
2. 源码运行： ./bootstrap_python312.sh 准备内嵌 Python 3.12 后，
    ./py312 run_tk.py 启动。

## 第 1 步：选择设备

界面是「节点面板」：默认已有 **录音输入 → AI 降噪 → 音频输出 → VU 电平表 → 频谱图** 五行。

1. **录音输入**：行内下拉选物理麦克风。Linux 按**声卡**枚举设备——
   一个声卡有多个**接口**（如板载声卡的数字麦 Mic1 / 模拟麦 Mic2 / 扬声器），
   各接口对应真实设备，在系统声卡设置（pavucontrol）里启用对应接口即可采集；
2. **音频输出**：选扬声器/耳机即可；想让其它软件使用降噪后的声音时，
   选 **PureVox 虚拟麦克风**（见下一步）；
3. **采样率**：PipeWire 统一重采样为 48kHz 单声道，无需手动设置。

## 第 2 步：虚拟麦克风（可选）

Linux 虚拟麦克风由 PureVox 经 PipeWire 创建，**启动时不会自动创建**：
**设置 ▾ → 虚拟声卡** 会在「创建」与「清理」之间切换（已创建则点击即清理，反之创建），
并弹框告知结果。创建后提供两个出口：

- **PureVox 虚拟麦克风**（purevox_out.monitor）—— 宽口径源，供绝大多数软件选用；
- **PureVox mic**（purevox_mic）—— 供 OBS 等只列"真源"的软件使用（由前者重映射而来）。

其它软件把输入设备设为「**PureVox 虚拟麦克风**」即可收到降噪后的声音；
不用时回 **设置 ▾ → 虚拟声卡** 点一下清理，创建/清理均安全幂等。

## 第 3 步：开始降噪

点击 **启动音频处理** 按钮，按钮变红即表示降噪已开启。

## 处理链与节点

与 Windows 版一致：全部处理以「节点」形式存在，用 **添加 ▾** 增加、拖拽排序、× 删除。

- **设备**：录音输入 / 网络输入 / 播放输入 / AEC 输入 / 音频输出
- **媒体输入**：音效板 / 音乐播放器 / 桌面声音输入
- **处理**：增益 / 自动增益 AGC / 噪声门 VAD / 均衡器 EQ（10·31·61 段）/ 压缩器 / AI 降噪 / 目标说话人 TSE
- **可视化**：VU 电平表 / 频谱图

默认链已含 **AI 降噪**。回声消除用 **添加 ▾ → 设备 → AEC 输入**（far 参考源：扬声器或另一路麦克风）；
目标说话人提取加 **目标说话人 TSE** 节点，先在它行内录制一段参考语音。

## 增益 / 均衡器 / VU

与 Windows 版完全一致：前增益（说话时 VU 峰值 **-12 ~ -6 dB** 最佳）、
AGC、VAD；均衡器在 **均衡器 EQ** 节点行内点 **均衡器编辑** 打开（10 / 31 / 61 段）。

## 远程麦克风（手机无线麦克风）

与 Windows 版步骤一致：**添加 ▾ → 设备 → 网络输入** → 行内选网卡（IP），
右侧显示二维码与监听地址 → 启动处理 → 手机扫码/打开网址，或手机装 App 自动发现并推流；
降噪后的音频输出到你选定的设备，想交给其它软件继续选「**PureVox 虚拟麦克风**」即可。

## 数据目录

配置、日志、录音文件均保存在 ~/.purevox/ 。

## 常见问题

**Q：完全没有声音？**

确认 **pipewire-pulse** 正在运行（ `systemctl --user status pipewire-pulse` ），再确认输入/输出设备下拉是否选对。

**Q：OBS 里选不到虚拟麦克风？**

OBS 只列"真源"：请选 **PureVox mic**，而不是 PureVox 虚拟麦克风（monitor 源）。

**Q：AEC 没有效果？**

AEC 行的 far 参考源要选对：用音响外放选扬声器，对照场景选另一路麦克风。far 参考缺失的行会被跳过并告警。

**Q：怎么卸载虚拟麦克风？**

**设置 ▾ → 虚拟声卡** 点一下即可（已创建时该操作为清理），删除安全幂等。
