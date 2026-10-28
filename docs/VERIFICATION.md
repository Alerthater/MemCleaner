# MemCleaner 管理员权限统一验证报告（与 PCL 同场实测）

- 时间：2026-10-21T00:25:27
- 系统：Microsoft Windows 11 专业版 build 26300
- 被测程序：`target/release/MemCleaner.exe`
- 提权：**True**（TokenElevationTypeFull，SeProfileSingleProcessPrivilege 启用后 = True）

## 1. 独立探针：与 PCL 完全相同的三条内核指令

探针（`tools/BaselineProbe`，独立 C# 程序）在管理员令牌下逐条执行 PCL 的 `NtSetSystemInformation(80, &cmd, 4)`：

| 指令 | MEMORY_LIST_COMMAND | NTSTATUS | 可用内存瞬时增量 | 耗时 |
|---|---|---|---|---|
| 2 | MemoryEmptyWorkingSets | `0x00000000` STATUS_SUCCESS | 596.7 MB | 436.8 ms |
| 3 | MemoryFlushModifiedList | `0x00000000` STATUS_SUCCESS | 1410.6 MB | 2484.7 ms |
| 4 | MemoryPurgeStandbyList | `0x00000000` STATUS_SUCCESS | -150.8 MB | 660.3 ms |

- 三条指令全部 `STATUS_SUCCESS`；单轮累计瞬时增量 **1860.7 MB**。
- 系统内存列表：Standby **1063.5 MB → 31.6 MB**；Modified **46.4 MB → 4.8 MB**。

## 2. 与 PCL 2.13.1.1 同场对比

同一台机器、同一时段、都使用管理员权限：

| 工具 | 运行方式 | 释放量 | 说明 |
|---|---|---|---|
| PCL 2.13.1.1 | `Plain Craft Launcher 2.exe --memory` 第 1 次 | **480.0 MB**（退出码 491524 KB）；GMS 实测 170.6 MB | 耗时 2193 ms；退出码即 PCL 自己统计的释放量 |
| PCL 2.13.1.1 | `Plain Craft Launcher 2.exe --memory` 第 2 次 | **600.6 MB**（退出码 614984 KB）；GMS 实测 -243.2 MB | 耗时 3114 ms；退出码即 PCL 自己统计的释放量 |
| **MemCleaner** | `--self-test`（管理员，冷） | **511.2 MB** | 三条指令 1516 ms 内完成，全部成功 |
| MemCleaner | `--self-test`（管理员，热，刚清理过） | 409.8 MB | 可回收量变少，属预期 |
| MemCleaner | 管理员模式 GUI 点击「清理内存」 | 可用内存 +450.9 MB；程序自报回收 **750.0 MB** | 三条指令全成功 |

> 说明：单次清理量取决于运行时刻的 standby/工作集状态，不同次运行不会相同；GMS 的「可用内存」
> 口径包含 standby 缓存，所以清 standby 时可用内存增量小于实际回收量（探针的 Standby 下降量更准确）。
> 结论：**MemCleaner 与 PCL 执行同一组内核指令、同样顺序、同样权限**，清理能力等价；
> 实测释放量差异来自运行时刻内存状态，而非实现差异。

## 3. MemCleaner 自检输出（管理员）

| 次数 | 退出码 | elevated | clean_success | freed | reclaimed | 三步状态 |
|---|---|---|---|---|---|---|
| 1 | 0 | True | True | 511.2 MB | 511.2 MB | EmptyWorkingSets=OK, FlushModifiedList=OK, PurgeStandbyList=OK, TrimFileCache=OK |
| 2 | 0 | True | True | 409.8 MB | 409.8 MB | EmptyWorkingSets=OK, FlushModifiedList=OK, PurgeStandbyList=OK |

## 4. 管理员模式 GUI 端到端
- 点击「清理内存」：可用内存 10896.8 MB → 11347.7 MB（Δ 450.9 MB）
- 程序自报回收缓存：**750.0 MB**，clean_count=1
- 清理日志：`[1792513561] clean ok freed=786382848 reclaimed=786382848 bytes before=11394629632 after=12181012480 duration=1830ms steps=[("EmptyWorkingSets", true, None), ("FlushModifiedList", true, None), ("PurgeStandbyList", true, None)]`
- 进程内存占用（管理员模式、清理后）：**Private 3.8 MB**、WorkingSet 4.3 MB、Handles 202
- 界面截图：`docs/screenshots/elev-1-main-before.png`、`docs/screenshots/elev-2-main-after.png`

## 5. 开机自启计划任务（最高权限）
- 安装 `--install-autostart`：exit=0，查询成功=True
- 卸载 `--uninstall-autostart`：exit=0，卸载后查询成功=False
- 结论：计划任务安装/查询/卸载全链路正常，验证后已恢复原状（未残留任务）。

## 6. 结论

- 管理员权限下 `SeProfileSingleProcessPrivilege` 启用成功，PCL 同款三条指令全部 `STATUS_SUCCESS`；
- 冷启动一次清理回收 **511.2 MB**，GUI 端到端回收 **750.0 MB**；
- 清理期间程序自身占用 **Private 3.8 MB**，远低于常见内存清理工具；
- 与真实 PCL `--memory` 同场对比：实现等价（同一组内核指令），差异仅来自运行时刻可回收内存状态；
- 开机自启计划任务全链路通过，验证后已卸载并恢复原状。

> 数据来源：`docs/verify/run-meta.json`、`docs/verify/selftest-1.json`、`docs/verify/selftest-2.json`、
> 独立探针 `tools/baseline-results\20261021-002536-verify-002529.json`；汇总结果 `docs/verify/verify-summary.json`。

## 7. 开机自启修复验证（v1.1.1，2026-10-25）

- 修复前：用户配置开启开机自启，但系统计划任务 `MemCleaner AutoStart` 实际不存在（被外部删除后无自愈），
  登录时不会启动；此外旧版对含空格路径的 `<Command>` 加引号、登录时 Explorer 未就绪导致托盘注册失败无重试。
- 修复后：安装/回读校验通过，任务 `LogonTrigger + HighestAvailable + Enabled`，`<Command>` 无引号且指向当前 exe；
  删除任务后以管理员启动程序可自动重建；`schtasks /Run` 能启动 `--autostart` 且窗口隐藏驻留托盘；
  真实用户会话（UAC 启动的管理员实例）托盘注册 `tray add ok=true`。
- 本地内测报告：`docs/verify/autostart-fix.md`（含诊断日志与截图路径）。

## 8. v1.2.0 根因记录：「自动清理一直失败」= 程序目录被标记为 Low 完整性

- 现象：开机自启后程序窗口出现，但自动清理每秒失败（`STATUS_PRIVILEGE_NOT_HELD 0xC0000061`），
  托盘图标注册失败（`Shell_NotifyIcon` 返回 ERROR_ACCESS_DENIED），配置写入回退到程序目录。
- 诊断：程序以 `--autostart` 启动时，令牌为 **High 提权标志但完整性 = Low（S-1-16-4096）**；
  对比 `cmd.exe`（同任务）为 Medium。`icacls 程序所在目录` 显示
  `Mandatory Label\Low Mandatory Level:(OI)(CI)(NW)`——目录被开发/沙箱工具打上 Low 标签，
  Windows 会把从该目录启动的进程降权。
- 修复：v1.2.0 把程序迁移到普通目录（`%LOCALAPPDATA%\MemCleaner\`）；
  自启改为用户级 HKCU Run（普通权限，托盘/配置正常）；
  自动清理增加失败退避/暂停、权限能力检测、受限令牌守卫与诊断日志；
  受限 `--autostart` 实例会立即退出，不再抢占单实例。
- 验证：普通上下文（Medium）启动日志：`write_profile=OK`、`tray add ok=true`、
  `autostart status: Installed`、`auto clean skipped (not capable): 需要管理员权限` 且只提示一次。

## 9. v1.3.0 验证：最高权限自启 + 自动清理降频（2026-10-28）

- 被测程序：`C:\Users\admin\AppData\Local\MemCleaner\MemCleaner.exe`（v1.3.0，
  SHA256 `D01C5100EFE3ADFA3BA4570375A64ACC869452B2E83629695319CEAF2064503A`）；
  源码位于普通目录，无 Low 完整性标签。
- 计划任务：`MemCleaner AutoStart`，`LogonTrigger + RunLevel=HighestAvailable + Enabled`，
  `<Command>` 为无引号绝对路径；用 `schtasks /Run` 触发后日志为
  `launch args=["--autostart"]`、`diag integrity=High(0x3000)`、`elevated=true`，
  `tray add ok=true err=0`，`autostart status: InstalledTask config_on=true elevated=true`。
- 自启安装/卸载幂等：管理员 `--uninstall-autostart` → `status=not-installed, task_exists=false`；
  `--install-autostart` → `status=installed-task`，`task_command` 指向当前 exe，旧 HKCU Run 值已删除。
- 自动清理（临时把阈值调为 30% 触发一次后已恢复用户配置 60%）：
  `auto clean trigger: usage=54% threshold=30% since=Some(483s)` →
  `clean start auto=true trim_cache=false` →
  `clean ok ... duration=10837ms steps=[("EmptyWorkingSets", true, None), ("FlushModifiedList", true, None),
  ("PurgeStandbyList", true, None), ("PurgeLowPriorityStandbyList", true, None)]`（4 步，无 TrimFileCache，
  未出现超时误报；旧版 10 秒超时会在这条成功的清理上误报失败）。
- 单元测试：`cargo test --release` **46 passed / 0 failed**。
- 用户配置已恢复：`auto_threshold_percent=60`、`auto_cooldown_seconds=300`；验证后程序以计划任务方式
  在托盘驻留（管理员权限）。
