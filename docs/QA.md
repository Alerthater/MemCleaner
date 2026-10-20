> 说明：本报告是项目开发期间在开发机上的验证记录，文中的绝对路径/进程号为当时的现场数据，复跑时请按你本机路径调整。

# MemCleaner QA 测试报告（最终发布构建）

- 报告日期：2026-10-20（最终发布构建复测：13:09–13:15）
- 测试人：qa-verify（Team Lead 的 QA 队友）
- 被测程序：MemCleaner 1.0.0（Rust / windows-sys 0.61.2 / 自绘 Win32 UI）
- 被测构建：`.\target\release\MemCleaner.exe`
  - 构建时间：2026-10-20 13:08:12（源码最后修改 13:07:57）
  - 大小：404,992 字节
  - SHA256：`F664B86AD49EFD4F54C0A9D242E2D019EA577C07C815F840C10BC7204DF9F17D`
  - 发布同步：`.\dist\MemCleaner.exe` 与本构建 **SHA256 完全一致**
- 本轮修复：`src/platform/single_instance.rs`（识别 `ERROR_ACCESS_DENIED`，原 Bug-01）、`src/config.rs`（新增 `Config::sanitize()`，原 Bug-02）
- 报告结论：**111/111 项断言全部通过（5 个自动化脚本）；Bug-01（高）、Bug-02（中）已修复并经复跑验证；无未解决的高/中严重度功能问题；Bug-03 为已确认的产品取舍（已知限制）。最终构建可发布。**

---

## 1. 环境

| 项目 | 值 |
|---|---|
| 操作系统 | Microsoft Windows 11 专业版 build 26300.9711（26H2） |
| 会话 | 同一个交互式会话（Session 1），UAC 已开启 |
| 当前用户/完整性 | `LAPTOP-TCDD3T8T\admin`，**标准用户（Medium 完整性，未提权）** |
| 显示器 / DPI | 2048×1152，工作区 2048×1104，系统 DPI 125%（`GetDpiForWindow=120`） |
| 窗口物理尺寸 | 500×700（400×560 逻辑像素） |
| Rust 工具链 | rustc / cargo 1.99.1（MSVC） |
| 构建目录 | 直接 `.\target\release`（**已解锁**；`cargo build --release` 无需 CARGO_TARGET_DIR） |
| 被试数据目录 | `MEMCLEANER_DATA_DIR=.\testdata\qa\...`（可写；日志 `MEMCLEANER_LOG=1`） |
| 既有实例 | 上轮遗留的 上一轮遗留的管理员实例已在本轮测试前结束；**本轮测试期间无任何 MemCleaner 实例常驻** |

> 历史说明：上一轮 QA 针对**修复前构建** `C2172C21…`（临时构建目录，含 Bug-01/Bug-02）。当时环境中存在一个旧的管理员实例（用于复现单实例边界场景）
> 持有同名互斥体，暴露了 Bug-01；越界配置用例暴露了 Bug-02。本轮针对修复后的最终发布构建做回归验证，
> 测试对象即 `target\release\MemCleaner.exe` 本体（非快照），测试前后 SHA256 一致。

---

## 2. 最终构建与修复验证

| 项目 | 结果 |
|---|---|
| `cargo build --release` | 成功（增量，构建前后 exe SHA256 不变） |
| `cargo test --release` | **20 / 20 通过**（新增 `config::tests::sanitize_fixes_out_of_range`） |
| 编译 warning | 19 条（bin；多为 dead code / 未使用字段，见 OBS-3） |
| `--version` | `MemCleaner 1.0.0` |
| `dist` 同步 | `dist\MemCleaner.exe` 与 `target\release\MemCleaner.exe` 字节一致（同 SHA256） |
| 修复 1 | `single_instance.rs::acquire()`：`CreateMutexW` 返回空句柄且 `GetLastError()==ERROR_ACCESS_DENIED(5)` 时视为“已有实例”（此前为 fail-open） |
| 修复 2 | `config.rs`：新增 `Config::sanitize()`，在 `load()` 与 `save()` 均调用；阈值仅允许 0 或 50..=100（否则回退 85），间隔 0..=1440 分钟，冷却 10..=3600 秒，历史字节数上限 2^50 |

---

## 3. 结果总览（对应任务测试项 1–9）

| # | 测试项 | 结果 | 关键数据/说明 |
|---|---|---|---|
| 1 | `cargo test --release` 全部通过 | ✅ 通过 | **20/20 通过**（含新增 sanitize 单测） |
| 2 | 内存占用基线（5 状态 × 3 采样） | ✅ 通过 | Private **3.46–3.48 MB**、WorkingSet 13.8–13.9 MB、Handles 181、GDI 25–30 |
| 3 | 泄漏检测（30 开关 / 50 显隐 / 20 清理取消） | ✅ 通过 | Private Δ **0.00 MB**、Handles Δ +1、GDI Δ 0、User Δ −1 → 无泄漏 |
| 4 | 单实例（应为单实例并唤醒） | ✅ 通过（Bug-01 已修复） | behavior 33/33：B 退出 exit=0、A 被唤醒、同 exe 进程数=1；专项分支测试 9/9（含修复前对照） |
| 5 | 配置健壮性（非法 JSON/缺字段/越界/只读目录） | ✅ 通过（Bug-02 已修复） | 4/4 用例通过；`threshold=250 → 85` 回退 | 
| 6 | `--self-test` / `--version` / `--help` | ✅ 通过 | 27/27 检查通过；exit=2、elevated=false、cmd2/cmd4 NTSTATUS=0xC0000061 |
| 7 | 托盘 / 窗口 / Esc / `minimize_to_tray=false` | ✅ 通过 | 11/11 检查通过 |
| 8 | `qa-ui-test.ps1 -Action full` 交互回归 | ✅ 通过 | 32/32 检查通过；config 变化符合预期 |
| 9 | 视觉截图 `docs/screenshots/qa-*.png` | ✅ 通过 | 18 张（最终构建重新生成） |
| 附 | `--autostart` 静默启动 | ✅ 通过 | 隐藏启动、进程存活、`WM_APP+1` 可唤起；Private 2.81 MB |
| 附 | 单实例修复分支专项（无管理员模拟） | ✅ 通过 | 9/9；`ACCESS_DENIED` 分支被真实触发并正确退出；修复前构建对照仍 fail-open |

**汇总：5 个自动化脚本共执行 32 + 10 + 33 + 27 + 9 = 111 项断言，全部通过（111/111，0 失败）。**

---

## 4. 详细结果

### 4.1 单元测试 `cargo test --release`

```
running 20 tests
test config::tests::sanitize_fixes_out_of_range ... ok
...（其余 19 项全部 ok）
test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

覆盖：配置默认值/越界清洗/历史记录、清理项映射、自动清理阈值/冷却/间隔、内存百分比换算、
指令常量 2/3/4/80 与 PCL 一致、GlobalMemoryStatusEx 数据、提权判定、计划任务 XML、UI 命中测试
（主界面/设置页 9 行/确认框）、格式化函数、设计常量。**QA 未修改 `src/`。**

历史：更早一轮（0:56 构建）曾失败 2 个用例，均为测试期望落后于实现，项目负责人已修正；当前 20/20。

### 4.2 内存占用基线（任务项 2）

口径：`PrivateMemorySize64`（任务管理器「提交大小」）、`WorkingSet64`（工作集）、`HandleCount`、
`GetGuiResources`（GDI/User 对象）。每状态稳定后采样 3 次取均值。

| 状态 | Private (MB) | WorkingSet (MB) | Handles | GDI | User | Threads |
|---|---:|---:|---:|---:|---:|---:|
| A 启动后空闲 10s | 3.48 | 13.84 | 181 | 25 | 14 | 4 |
| B 打开设置页 | 3.48 | 13.86 | 181 | 29 | 14 | 4 |
| C 打开清理确认框 | 3.48 | 13.87 | 181 | 30 | 14 | 4 |
| D 隐藏到托盘 | 3.48 | 13.88 | 181 | 30 | 14 | 4 |
| E `WM_APP+1` 重新显示 | 3.47 | 13.88 | 181 | 30 | 15 | 4 |
| `--autostart` 静默启动（隐藏未首绘，10s） | 2.81 | 9.45 | 148 | – | – | – |

- 常驻内存极低：**Private 稳定 3.46–3.48 MB**，满足“常驻 < 6 MB”目标；隐藏启动（无首帧绘制）约 2.81 MB。
- 打开设置页/确认框仅增加 GDI 字体缓存（+4～5 个 GDI 对象），之后不再增长。
- 对照：旧 UI 构建（`ACA361A4…`）为 Private 5.03–5.09 MB / WS 17.3–17.5 MB；新 UI 修复前构建（`C2172C21…`）
  为 3.48 MB / 13.7–13.9 MB —— **最终构建与修复前持平，修复未引入常驻开销**。
- 原始数据：`testdata\qa\memory-test-results.json`、`testdata\qa\final-memory.log`

### 4.3 泄漏检测（任务项 3）

| 阶段 | Private (MB) | WorkingSet (MB) | Handles | GDI | User | Threads |
|---|---:|---:|---:|---:|---:|---:|
| L0 基线 | 3.46 | 13.88 | 181 | 30 | 15 | 4 |
| L1 设置页开关 ×30 | 3.46 | 13.89 | 182 | 30 | 14 | 4 |
| L2 显示/隐藏 ×50 | 3.46 | 13.89 | 182 | 30 | 15 | 4 |
| L3 清理→取消 ×20 | 3.46 | 13.90 | 182 | 30 | 14 | 4 |
| **L3 − L0** | **0.00** | +0.02 | **+1** | **0** | **−1** | **0** |

判定阈值：Private Δ ≤ 1 MB、Handles Δ ≤ 6、GDI Δ ≤ 4、User Δ ≤ 4。

- 结论：**未发现内存/句柄/GDI/User 对象泄漏**；100 次批量交互后线程数保持 4，无增长。
- 配置连续保存 30 次无崩溃，日志从 719 字节正常增长到 14,615 字节。

### 4.4 单实例（任务项 4）—— ✅ 通过（Bug-01 已修复）

**A. 普通双实例 + 唤醒路径（behavior 33/33 中的 single 6/6）**

```
B: exited=True exitCode=0; A: visible=True; 同 exe 进程数=1
[PASS] single:second-instance-exits
[PASS] single:second-instance-exit-code-zero
[PASS] single:first-window-woken
[PASS] single:only-one-process count=1
```

- 实例 A 隐藏到托盘后启动实例 B：**B 立即退出（exit code 0），A 被唤醒显示，同 exe 进程数 = 1**。

**B. `ERROR_ACCESS_DENIED` 修复分支专项测试（`tools\qa-single-instance-branch-test.ps1`，9/9）**

Bug-01 的真实触发条件是“高完整性（管理员）实例持有互斥体 → 普通实例 `CreateMutexW` 返回
`ERROR_ACCESS_DENIED(5)`”。本机当前无管理员权限可用，故用**空 DACL 命名互斥体**（SDDL `"D:"`，
对所有人拒绝访问）稳定复现等价的 `ACCESS_DENIED` 条件，直接覆盖修复分支：

| 步骤 | 实测 |
|---|---|
| 创建空 DACL 互斥体 | `handle=5272 lastErr=0` |
| 普通进程打开该互斥体 | `handle=0 lastErr=5`（= `ERROR_ACCESS_DENIED`，条件成立） |
| **最终构建**在此条件下启动 | **立即退出：exited=True、exitCode=0、无窗口（hwnd=0）、无 `app started` 日志** ✅ |
| **修复前构建**（`C2172C21…`）对照 | 仍然 fail-open：窗口创建并存活（证明该分支确实被触发，测试有效） |
| 释放互斥体后最终构建再启动 | 正常：窗口可见、日志 `app started` ✅ |

- 修复代码：`acquire()` 在 `CreateMutexW` 返回空句柄且 `GetLastError()==ERROR_ACCESS_DENIED` 时返回 `None`
  （调用方即唤醒已有实例后退出）；仅其它未知错误保留 fail-open，避免完全无法启动。
- 证据：`testdata\qa\single-instance-branch-results.json`、`final-single-branch.log`、`final-behavior.log`
- 说明：真实“管理员实例 + 普通实例”的端到端验证仍需管理员权限（见 §7 限制）；本测试以等价访问拒绝条件
  覆盖了修复分支语义，且普通双实例/唤醒路径已实测通过。

### 4.5 配置健壮性（任务项 5）—— ✅ 4/4 通过（Bug-02 已修复）

每个用例：写入指定 `config.json` → 启动 → 检查窗口/进程/系统弹窗 → 通过 UI 触发保存 → 读回 JSON。

| 用例 | 结果 | 实测 |
|---|---|---|
| a) 非法 JSON `{ this is not valid json !!` | ✅ | 回退全默认（threshold=85、working_sets=true、minimize_to_tray=true），无崩溃/无系统弹窗 |
| b) 缺字段 `{"theme":"light"}` | ✅ | 缺失字段补默认；`theme=light` 保留 |
| c) 越界 `auto_threshold_percent=250` | ✅ **已修复** | **`threshold 250 → 85`**（`config::sanitize` 在 load/save 均生效）；设置页显示“超过 85% 时自动清理 / 当前 85%”（截图 `qa-beh-06-config-threshold-250.png`） |
| d) 只读目录（`icacls /deny W`） | ✅ | 保存失败但进程不崩溃、config 未被半写破坏、无 `config.json.tmp` 残留、无系统错误框 |

- 新增单测 `config::tests::sanitize_fixes_out_of_range` 覆盖 250→85、99999→0、0→60 等回退规则。
- 证据：`testdata\qa\behavior-test-results.json`（cfg-* 16/16）、`qa-beh-06-config-threshold-250.png`

### 4.6 命令行 / 自检模式（任务项 6）—— ✅ 27/27

| 检查 | 结果 |
|---|---|
| `--version` | exit 0，输出 `MemCleaner 1.0.0` |
| `--help` | exit 0，含“用法”、`--self-test`、`--version`、`--install-autostart`、`--uninstall-autostart` |
| `--self-test --out <json>` | **exit 2**（普通权限），JSON 已写入且可解析 |
| JSON 字段 | `tool=MemCleaner`、`version=1.0.0`、**`elevated=false`**、`os_build=26300`、`clean_success=false`、`duration_ms=41` |
| JSON 扩展字段 | `reclaimed_bytes`、`standby_before/after_bytes`、`modified_before/after_bytes` 均存在（standby≈5.89 GB、modified≈18.2 MB） |
| `EnableSeProfileSingleProcessPrivilege` | success=false，error=**1300**（ERROR_NOT_ALL_ASSIGNED） |
| `EmptyWorkingSets`（cmd 2） | success=false，error=**0xC0000061** ✅ |
| `FlushModifiedList`（cmd 3） | **success=true，error=null**（标准用户下实际成功，见 OBS-1） |
| `PurgeStandbyList`（cmd 4） | success=false，error=**0xC0000061** ✅ |
| `--self-test`（不带 --out） | exit 2，stdout 仍输出 JSON |
| 进程/窗口残留 | 无残留、未创建窗口、无 `#32770` 系统对话框 |
| 证据 | `testdata\qa\cli-test-results.json`、`testdata\qa\cli\selftest.json` |

### 4.7 托盘 / 窗口 / Esc 行为（任务项 7）—— ✅ 11/11

| 检查 | 结果 |
|---|---|
| 点 ✕ 后隐藏到托盘 | 进程存活、`IsWindowVisible=false` |
| `WM_APP+1 (0x8001)` 重新显示 | 可见=true；与隐藏前截图差异 0.06% |
| 设置页按 Esc | 回到主界面（与主界面截图差异 0.09%，与设置页差异 99.31%） |
| 确认框按 Esc | 关闭（遮罩消失） |
| 确认框不是系统 MessageBox | 全程无 `#32770` 窗口 |
| 主界面按 Esc（minimize_to_tray=true） | 隐藏到托盘 |
| `minimize_to_tray=false` 时点 ✕ | 进程退出，exit code **0** |

### 4.8 UI 交互回归（任务项 8）—— ✅ 32/32

`tools\qa-ui-test.ps1 -Action full` 对最终构建执行，32 项断言全部通过，关键项：

| 检查 | 实测 |
|---|---|
| 打开设置页 | 与主界面截图差异 29.83% |
| 切换“清空工作集” | `config.json` 中 `clean_working_sets` true → **false** |
| 主题循环 2 次 | `theme`: system → dark → **light**（持久化正确） |
| 点击清理（普通权限） | 应用内确认框出现（四角亮度 248.75 → 168.5） |
| 点“取消” | 确认框消失、日志记录 `ConfirmCancel`、未发生清理（clean_count=0） |
| ✕ 隐藏 / `WM_APP+1` 显示 | 均通过 |
| Esc 关设置页 / 主界面 Esc 隐藏 | 均通过 |
| 最终 config | theme=light、clean_working_sets=false、auto_threshold_percent=85、clean_count=0 |
| 日志 | 含 `handle_action OpenSettings`、`CycleTheme`×2，无异常 |

### 4.9 补充：`--autostart` 静默启动

`MemCleaner.exe --autostart`：主窗口存在但不可见（托盘静默），进程存活；启动 10s 后
Private 2.81 MB / WorkingSet 9.45 MB / Handles 148；`WM_APP+1` 后窗口正常显示。
（未验证计划任务创建/登录自启链路，需管理员权限，见 §7。）

### 4.10 修复前后对照（最终构建 vs 修复前构建）

| 项目 | 修复前 `C2172C21…` | 最终 `F664B86A…` |
|---|---|---|
| 单实例：第二实例退出 | ❌ 否（两实例并存） | ✅ exit code 0 |
| 单实例：第一窗口唤醒 | ❌ 否 | ✅ 是 |
| 同 exe 进程数 | 2 | 1 |
| `threshold=250` 保存后 | 250（原样持久化） | **85（回退默认）** |
| behavior 脚本 | 29/33 | **33/33** |
| 空 DACL（ACCESS_DENIED）分支 | fail-open 启动窗口 | **立即退出，不建窗口** |
| 内存基线（Private） | 3.48 MB | 3.46–3.48 MB（持平） |

---

## 5. Bug 清单（修复状态）

### Bug-01（高）单实例失效 —— ✅ **已修复（可关闭）**

- 原问题：管理员实例常驻托盘时，普通权限再次启动会出现第二个实例且不唤醒（`CreateMutexW` 返回
  `ERROR_ACCESS_DENIED(5)`，旧代码只识别 `ERROR_ALREADY_EXISTS(183)` → fail-open）。
- 修复：`src/platform/single_instance.rs::acquire()` 识别 `ERROR_ACCESS_DENIED` 为“已有实例”，
  返回 `None` 触发 `wake_existing_instance()` 后退出；仅未知错误保留 fail-open。
- 修复后证据：
  1. behavior `single` 6/6 通过：B exit=0、A 唤醒、进程数=1；
  2. 专项分支测试 9/9：空 DACL 互斥体（普通进程打开得 `lastErr=5`）下，**最终构建立即退出（exit 0、无窗口、无日志）**，
     修复前构建对照仍 fail-open 启动窗口（证明分支被真实触发）；
  3. 释放互斥体后最终构建正常启动。
- 证据文件：`testdata\qa\behavior-test-results.json`、`single-instance-branch-results.json`、
  `testdata\qa\final-behavior.log`、`testdata\qa\final-single-branch.log`
- 遗留验证：真实“管理员实例 + 普通实例”端到端需管理员权限（见 §7）；修复分支已用等价条件覆盖。

### Bug-02（中）配置越界值未校验 —— ✅ **已修复（可关闭）**

- 原问题：`threshold=250` 原样生效并持久化，自动清理阈值被静默禁用。
- 修复：`Config::sanitize()`（load/save 均调用）：阈值仅允许 0 或 50..=100，否则回退 85；
  间隔 >1440 → 0；冷却不在 10..=3600 → 60；历史字节上限 2^50。新增单元测试。
- 修复后证据：behavior `cfg-range` 4/4 通过（`threshold 250 -> 85`）；截图
  `qa-beh-06-config-threshold-250.png`（设置页显示 85%）；未崩溃、无系统弹窗。
- 证据文件：`testdata\qa\behavior-test-results.json`、`testdata\qa\final-behavior.log`

### Bug-03（低）设置页无 `minimize_to_tray` / `animations` 入口 —— 📌 **已知限制（产品取舍，不修复）**

- 产品决策（已确认）：设置页保持 **9 行简洁布局**（自动清理、触发阈值、5 个清理项、主题、开机自启），
  **不新增入口**；`minimize_to_tray`、`animations` 两个配置项仅在 `config.json` 中手动修改。
- 现状说明：
  - `minimize_to_tray` 的功能本身正常（点 ✕ 隐藏/退出行为已由 behavior 11/11 验证），只是无 UI 开关；
  - `animations` 字段当前无消费逻辑，手动改为 `false` 也不会改变动画行为；
  - `ui::hit_test` 设置页只遍历 9 行，`Action::ToggleMinimizeToTray/ToggleAnimations` 为不可达代码。
- 影响：不影响核心功能与发布；记录为已知限制。
- 相关文案问题（建议修正，见 OBS-6）：`dist\使用说明.md` 仍写“设置页里可以…关闭窗口时最小化到托盘”，
  与上述产品取舍不一致。

---

## 6. 观察项（非阻塞，建议关注）

| 编号 | 观察 | 说明/建议 |
|---|---|---|
| OBS-1 | `FlushModifiedList`（cmd3）在标准用户下返回**成功**（error=null） | 任务验收只要求 cmd2/cmd4 为 `0xC0000061`，本项通过；但 `docs/pcl-reverse-engineering.md` 与 `--self-test` 注释中“三条指令都需要 SeProfileSingleProcessPrivilege”的表述与实测不符，建议修正文档措辞 |
| OBS-2 | 线程数：最终轮全程 4 并保持稳定 | 个别轮次首次批量交互后短暂 4→5（系统线程池惰性创建），100+ 次交互后不再增长，非泄漏 |
| OBS-3 | release 构建有 **19 条 warning**（dead code、未使用字段/方法等） | 建议发布前清理（`cargo fix` + 删除未用字段），降低维护噪音；不影响功能 |
| OBS-4 | `--install-autostart` / `--uninstall-autostart` 未实测 | 需要管理员权限且会修改系统计划任务；本次只验证 `--help` 中出现。建议由 UAC 验证脚本统一覆盖 |
| OBS-5 | `dist\MemCleaner-fallback.exe` 仍是**修复前旧构建** | 实测其 SHA256 = `ACA361A4…`（2026-10-20 00:56:07，旧 UI，含 Bug-01/Bug-02）。若该 fallback 仍要随包发布，建议用最终源码重新构建；否则建议从 dist 移除 |
| OBS-6 | `dist\使用说明.md` 与产品现状不一致 | ① 第二节/第三节写“设置页里可以关闭窗口时最小化到托盘”，实际无该 UI 入口（仅 config.json）；② 写“窗口会显示『已清理 X 缓存，可用内存 +Y』”，实际提示文案为“已释放 X 内存”。建议修正文案后再发布 |
| OBS-7 | 环境已清理 | 上一轮遗留的管理员实例已结束，`target\release` 可正常构建；真实高完整性端到端验证建议由负责人用 UAC 脚本补一次（低风险，修复分支已模拟验证） |

---

## 7. 测试限制

1. **未测试管理员权限路径**：真实清理效果（清空工作集/刷新已修改页/清待机列表）、提权重启、
   `--install-autostart`、真实“管理员实例 + 普通实例”唤醒等需要 UAC 的项，按约束未执行；
   单实例 `ERROR_ACCESS_DENIED` 修复分支已用等价 DACL 条件覆盖（§4.4B）。
2. 直接测试 `target\release\MemCleaner.exe` 本体，测试前后 SHA256 一致；`dist\MemCleaner.exe` 同哈希。
3. 5 个脚本**必须串行执行**：脚本启动时会清理“同 exe 路径”的陈旧测试实例以保证隔离；并发会互相干扰。
4. 只读目录用例通过 `icacls <dir> /deny <user>:(W)` 模拟，用例结束后已恢复 ACL。
5. 窗口截图使用 `PrintWindow(PW_RENDERFULLCONTENT)`，尺寸 500×700（125% DPI）；隐藏状态不截图。
6. 本报告“通过/失败”严格按任务描述验收；未覆盖的新增 CLI 行为只做记录、不擅自判定。
7. 测试期间未修改 `src/`。

---

## 8. 截图清单（`.\docs\screenshots\qa-*.png`，共 18 张，500×700）

> 均为**最终发布构建**（F664B86A…）于 2026-10-20 13:09–13:10 重新生成。

| 文件 | 内容 | 对应测试 |
|---|---|---|
| `qa-01-main.png` | 主界面（默认跟随系统 → 深色） | §4.8 |
| `qa-02-settings.png` | 设置页（深色，9 行） | §4.8 |
| `qa-03-settings-light.png` | 设置页（浅色，主题循环后） | §4.8 |
| `qa-04-main-light.png` | 主界面（浅色） | §4.8 |
| `qa-05-confirm-clean.png` | 清理确认框（应用内对话框） | §4.8 |
| `qa-06-after-cancel.png` | 取消确认框后回到主界面 | §4.8 |
| `qa-07-reshown.png` | ✕ 隐藏后 `WM_APP+1` 重新显示 | §4.8 / §4.7 |
| `qa-08-after-esc.png` | 设置页按 Esc 后回到主界面 | §4.7 |
| `qa-mem-01-idle.png` | 启动空闲 10s（内存基线采样时） | §4.2 |
| `qa-mem-02-settings.png` | 设置页（基线采样时） | §4.2 |
| `qa-mem-03-confirm.png` | 确认框（基线采样时） | §4.2 |
| `qa-mem-05-reshown.png` | 重新显示（基线采样时） | §4.2 |
| `qa-beh-01-main.png` | 行为测试主界面 | §4.7 |
| `qa-beh-02-settings.png` | 行为测试设置页（Esc 前） | §4.7 |
| `qa-beh-03-after-esc.png` | 设置页 Esc 后 | §4.7 |
| `qa-beh-04-confirm.png` | 行为测试确认框（Esc 前） | §4.7 |
| `qa-beh-05-after-esc-confirm.png` | 确认框 Esc 后 | §4.7 |
| `qa-beh-06-config-threshold-250.png` | **Bug-02 修复验证**：写入 `threshold=250` 后启动，设置页显示已回退 **85%**（“超过 85% 时自动清理”“当前 85%”） | §4.5 |

> `qa-06-after-cancel.png` 与 `qa-07-reshown.png` 内容相同（取消确认框后与重新显示后画面一致，属预期）。

---

## 9. 复跑命令

前置约定（每个 PowerShell 调用都是新进程；被测程序为 GUI 子系统，需走 `powershell.exe -File`）：

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
Set-Location .
$ps  = 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe'
$Exe = '.\target\release\MemCleaner.exe'
```

### 9.1 构建与单元测试

```powershell
cargo build --release        # 无需 CARGO_TARGET_DIR（target\release 已解锁）
cargo test --release         # 期望：20 passed; 0 failed
Get-FileHash $Exe -Algorithm SHA256
# 当前期望 SHA256 = F664B86AD49EFD4F54C0A9D242E2D019EA577C07C815F840C10BC7204DF9F17D
Get-FileHash .\dist\MemCleaner.exe -Algorithm SHA256   # 应与上一致
```

### 9.2 五个自动化测试（**串行**执行，全部 exit 0）

```powershell
# 9.2.1 UI 交互回归 + 状态截图（期望 32/32）
& $ps -NoProfile -ExecutionPolicy Bypass -File tools\qa-ui-test.ps1 -Action full -Exe $Exe

# 9.2.2 内存基线 + 泄漏（期望 10/10；约 75s）
& $ps -NoProfile -ExecutionPolicy Bypass -File tools\qa-memory-test.ps1 -Exe $Exe

# 9.2.3 单实例 / 托盘 / Esc / 配置健壮性（期望 33/33）
& $ps -NoProfile -ExecutionPolicy Bypass -File tools\qa-behavior-test.ps1 -Exe $Exe

# 9.2.4 CLI / --self-test（期望 27/27）
& $ps -NoProfile -ExecutionPolicy Bypass -File tools\qa-cli-test.ps1 -Exe $Exe

# 9.2.5 单实例修复分支专项（期望 9/9；含修复前构建对照）
& $ps -NoProfile -ExecutionPolicy Bypass -File tools\qa-single-instance-branch-test.ps1 -Exe $Exe
```

> 脚本默认 `-ProtectPid <旧实例PID>`（若该 PID 存在则永不结束它）；如环境变化可通过参数修改。
> 所有脚本为 **UTF-8 with BOM**；默认数据目录 `.\testdata\qa\...`，
> 结果写 `.\testdata\qa\*-test-results.json`，截图写 `.\docs\screenshots`。

### 9.3 结果文件（本轮最终构建）

| 文件 | 内容 |
|---|---|
| `testdata\qa\ui-test-results.json` | UI 回归 32 项断言 + 构建 SHA |
| `testdata\qa\memory-test-results.json` | 9 组状态采样 + 10 项断言 |
| `testdata\qa\behavior-test-results.json` | 单实例/托盘/配置 33 项断言 |
| `testdata\qa\cli-test-results.json` | CLI/自检 27 项断言 + 完整 self-test JSON |
| `testdata\qa\single-instance-branch-results.json` | Bug-01 修复分支专项 9 项断言（含修复前对照） |
| `testdata\qa\final-*.log` | 各套件原始控制台输出（ui/memory/behavior/cli/single-branch） |
| `testdata\qa\cli\selftest.json` | `--self-test` 原始 JSON 输出 |
| `testdata\qa\bin\C2172C21\MemCleaner.exe` | **修复前**构建（仅作对照，SHA256 `C2172C21…`） |

---

## 10. QA 交付物

| 文件 | 说明 |
|---|---|
| `docs/qa-report.md` | 本报告（最终发布构建版） |
| `tools/qa-ui-test.ps1` | UI 回归/截图脚本：默认 临时构建目录→可 `-Exe` 覆盖、`-KillAtEnd`（默认开）+ try/finally、只结束自己启动的 PID（保护指定 PID）、陈旧实例清理、断言/JSON/退出码 |
| `tools/qa-memory-test.ps1` | 5 状态 × 3 采样内存基线 + 3 组泄漏压测（30/50/20 次）+ GDI/User 计数 |
| `tools/qa-behavior-test.ps1` | 单实例、托盘/窗口/Esc、`minimize_to_tray=false`、4 类配置健壮性 |
| `tools/qa-cli-test.ps1` | `--version`/`--help`/`--self-test` 校验（exit code、JSON 字段、NTSTATUS） |
| `tools/qa-single-instance-branch-test.ps1` | **新增**：Bug-01 修复分支专项（空 DACL 模拟 `ERROR_ACCESS_DENIED`，含修复前构建对照，无需管理员） |
| `tools/qa-single-instance-probe/` | 历史根因最小复现工程（修复前证据，需高完整性实例占用互斥体才会复现 `last_err=5`） |
| `docs/screenshots/qa-*.png` | 18 张最终构建状态截图（§8） |

---

## 11. 结论与建议

1. **可发布性**：最终构建 111/111 项断言通过；核心功能（UI/托盘/Esc/配置/CLI 自检）、常驻内存
   （Private 3.46–3.48 MB）、无泄漏、单实例（含修复分支）、配置健壮性均达标。
2. **Bug 状态**：Bug-01（高）、Bug-02（中）已修复并经复跑验证，可关闭；Bug-03（低）为已确认的产品取舍，
   作为已知限制记录（设置页保持 9 行，`minimize_to_tray`/`animations` 仅 config.json 可改，不新增入口）。
3. **发布前建议（非阻塞）**：
   - OBS-5：`dist\MemCleaner-fallback.exe` 仍是修复前旧构建（`ACA361A4…`，含 Bug-01/Bug-02）——建议重新构建或移除；
   - OBS-6：`dist\使用说明.md` 与产品现状不一致（“设置页可关闭最小化到托盘”、清理完成提示文案），建议修正；
   - OBS-3：清理 19 条编译 warning；OBS-1：修正 PCL 文档中“三条指令都需要特权”的表述。
4. **最终管理员权限验证**（真实清理效果、提权重启、计划任务自启、真实高完整性实例唤醒）需在 UAC 环境
   由负责人统一执行；本报告已在 §7 明确边界，且修复分支已用等价条件验证。
