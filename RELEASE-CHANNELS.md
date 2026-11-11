# RELEASE-CHANNELS — 桌面 / 安卓独立发版与提交原则

> **这是硬约束，不是建议。** `AGENTS.md` 只保留索引和一句话；判定细节以本文件为准。
> 每次提交、每次发版前都要按这里检查。改这条流水线之前先改本文件。

## 0. 一句话

**一个仓库，两条发布通道**：

| 通道 | tag | 工作流 | 更新检查 | 更新日志范围 | 产物 |
| --- | --- | --- | --- | --- | --- |
| 桌面（当前：便携测试） | `v<yyyy.MM.dd.HHmm>` | `.github/workflows/release.yml` | 无（手动下载 zip） | 除 `android/` 外的提交 | Windows 便携 zip |
| 桌面（安装包恢复后） | `v*` | 同上 | `src-tauri/src/update/` | 同上 | exe / AppImage / deb / rpm |
| 安卓 | `android-v<yyyy.MM.dd.HHmm>` | `.github/workflows/android-release.yml` | `UpdateChecker`（只认 `android-v*`） | 只列 `android/` 下的提交 | 签名 APK |

两条通道各自建 GitHub Release、各自写更新日志、各自检查更新。桌面发版**不会**让手机弹出
「有新版本」，安卓发版也**不会**污染桌面的 release 列表。

## 1. 为什么不开长期分支

不为安卓单开分支。理由：

- **代码并没有分叉**：协议（`NETWORK.md`）、版本推导（`version.py`）、文档、工具链都是
  共用的。开分支等于把整棵树复制一份，之后每次改协议都要 cherry-pick，漂移迟早发生；
- **发版隔离靠 tag，不靠分支**：GitHub Release、更新检查、更新日志都能按 tag 前缀和路径
  区分，不需要分支这一层；
- **分支会让「谁是主线」变得模糊**：安卓的修复要不要回主干、桌面的协议改动要不要同步到
  安卓分支——这些都是可以避免的长期负担。

需要「只发安卓」时：从同一个 `main` 打 `android-v*` tag 即可。需要「只改安卓」时：只改
`android/` 下的文件并提交（见 §3 的路径纪律）。

**唯一例外**：如果将来出现「安卓要长期停更、桌面继续演进」这种情况，才讨论分支；
现在不是。

## 2. 发版流程

### 2.1 桌面

```bash
# 1) 门禁（至少 fmt / clippy / build / test / version / bundle）
python tools/automation/check.py

# 2) 打 tag（时间戳 = 本次发布时刻，UTC）
git tag v2026.11.12.1015
git push origin v2026.11.12.1015
```

`release.yml` 会构建 Windows 便携 zip、生成**桌面范围**的更新日志（排除 `android/`）、
创建 `v*` release。`PLATFORM=portable` 传给 `release_stage.sh`，只认一个 zip。
安装包流水线恢复后改回 4 件产物（`PLATFORM=desktop`）。

### 2.2 安卓

```bash
# 1) 只跑安卓自己的编译检查（不需要等桌面流水线）
cd android && ./gradlew :app:assembleDebug

# 2) 打安卓 tag
git tag android-v2026.11.12.1015
git push origin android-v2026.11.12.1015
```

`android-release.yml` 会构建签名 APK、生成**安卓范围**的更新日志（只列 `android/`）、
创建 `android-v*` release。手机端 `UpdateChecker` 只筛 `android-v*`，桌面 release
再新也不会被它当成「安卓更新」。

### 2.3 同一天发布两次

tag 的 `HHmm` 区分；显示版本 `yy.M.d` 可以重复，但 tag 必须唯一且按时间全序。
`versionCode = yyMMddHHmm ÷ 10`，仍然单调递增（同一 10 分钟内两次发版会撞号，
一次 CI 就要 20 分钟，实际不会发生）。

## 3. 提交原则（每次 commit）

1. **一个提交只做一件事。** 不要把「桌面顺手改的」和「安卓的改动」混在一起；
   也不要把无关重构塞进功能提交（更新日志是按提交标题生成的，混在一起用户就看不懂）。
2. **scope 写平台**（conventional commits 的 scope）：
   - `feat(android): ...` / `fix(android): ...` —— 只改 `android/`；
   - `feat(desktop): ...` / `fix(desktop): ...` —— 只改桌面（`src-tauri/` 等）；
   - `feat(core): ...` / `fix(core): ...` —— 两边共享（协议、工具链、共享文档）；
   - `feat(ui): ...` / `fix(ui): ...` —— 桌面界面；
   - `docs:` / `chore:` / `ci:` —— 不面向用户，更新日志里尽量少出现。
3. **路径要诚实**：更新日志按**路径**筛，不按提交前缀猜。
   - 安卓专属代码放 `android/`；
   - 桌面专属代码放 `src-tauri/` 等；
   - 共享协议 / 文档放两边都会读的位置（`NETWORK.md`、`AGENTS.md` 等），两边日志都会出现
     —— 这是正确的，因为两边真的都受影响。
4. **协议兼容**：改手机 ⇄ 电脑协议时必须同一次提交更新 `NETWORK.md` +
   `android/.../Proto.kt` 的版本号 + Rust 侧协议版本；否则一端更新后会连不上。
5. **发版 tag 前**：确认没有把临时调试、未完成的跨端改动带进 tag；安卓发版前
   `./gradlew :app:assembleDebug` 必须过，桌面发版前 `check.py` 必须过。
6. **不要为了「让日志好看」而拆假提交**：日志是给用户看变更的，不是营销文案；
   一个真实的改动就写真实的一句话。

## 4. 更新检查（两条链，各自只认自己的 tag）

- **桌面**：`src-tauri/src/update/mod.rs` 请求 GitHub `releases/latest`，只接受 `v*`
  （`tag_to_stamp` 会拒绝 `android-v*`）；`ASSET_SUFFIXES` 按平台匹配安装包。
- **安卓**：`UpdateChecker` 拉一页 release 列表，只筛 `android-v*`，取 `versionCode`
  最大的那条，再取 `*.apk` 资产。**不能**用 `/releases/latest`：桌面的新 release 会排在
  前面，手机就会看到一个装不了/不该装的「新版本」。

## 5. 更新日志

`tools/automation/release_notes.py` 是唯一实现：

```bash
# 桌面（排除 android/）
python tools/automation/release_notes.py --platform desktop --tag v... --out notes.md

# 安卓（只取 android/）
python tools/automation/release_notes.py --platform android --tag android-v... --out notes.md
```

上一个 tag 的匹配也按通道走：桌面 `v*`，安卓 `android-v*`。这样安卓的日志不会把桌面
提交算进来，反之亦然。

## 6. 发布前检查清单

- [ ] 改动只落在自己通道的目录里（或明确是 `core` 共享改动）；
- [ ] 提交标题的 scope 与平台一致；
- [ ] 桌面：`python tools/automation/check.py` 全绿；
- [ ] 安卓：`cd android && ./gradlew :app:assembleDebug` 通过；
- [ ] 协议相关改动：`NETWORK.md` + 两端版本号同步更新；
- [ ] tag 前缀正确（桌面 `v`，安卓 `android-v`），时分唯一；
- [ ] 发版后检查 release 只有本通道的产物，更新日志只列本通道的提交；
- [ ] CI 失败 / 从未生成 release 的 tag 立即删除（否则会截断下一次的日志范围）。
