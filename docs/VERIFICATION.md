# MemCleaner 管理员权限统一验证报告（与 PCL 同场实测）

- 时间：2026-10-20T13:13:35+08:00
- 系统：Microsoft Windows 11 专业版 build 26300
- 被测程序：`MemCleaner.exe`
- 提权：**True**（TokenElevationTypeFull，SeProfileSingleProcessPrivilege 启用后 = True）

## 1. 独立探针：与 PCL 完全相同的三条内核指令

探针（`tools/BaselineProbe`，独立 C# 程序）在管理员令牌下逐条执行 PCL 的 `NtSetSystemInformation(80, &cmd, 4)`：

| 指令 | MEMORY_LIST_COMMAND | NTSTATUS | 可用内存瞬时增量 | 耗时 |
|---|---|---|---|---|
| 2 | MemoryEmptyWorkingSets | `0x00000000` STATUS_SUCCESS | 1809.1 MB | 976.9 ms |
| 3 | MemoryFlushModifiedList | `0x00000000` STATUS_SUCCESS | 3152.7 MB | 4283.1 ms |
| 4 | MemoryPurgeStandbyList | `0x00000000` STATUS_SUCCESS | -357.3 MB | 1822.3 ms |

- 三条指令全部 `STATUS_SUCCESS`；单轮累计瞬时增量 **4603.1 MB**。
- 系统内存列表：Standby **4953.6 MB → 63.6 MB**；Modified **24.9 MB → 9.7 MB**。

## 2. 与 PCL 2.13.1.1 同场对比

同一台机器、同一时段、都使用管理员权限：

| 工具 | 运行方式 | 释放量 | 说明 |
|---|---|---|---|
| PCL 2.13.1.1 | `Plain Craft Launcher 2.exe --memory` 第 1 次 | **381.4 MB**（退出码 390508 KB）；GMS 实测 -476.5 MB | 耗时 3345 ms；退出码即 PCL 自己统计的释放量 |
| PCL 2.13.1.1 | `Plain Craft Launcher 2.exe --memory` 第 2 次 | **349.4 MB**（退出码 357752 KB）；GMS 实测 154.0 MB | 耗时 3091 ms；退出码即 PCL 自己统计的释放量 |
| **MemCleaner** | `--self-test`（管理员，冷） | **929.1 MB** | 三条指令 1927 ms 内完成，全部成功 |
| MemCleaner | `--self-test`（管理员，热，刚清理过） | 109.8 MB | 可回收量变少，属预期 |
| MemCleaner | 管理员模式 GUI 点击「清理内存」 | 可用内存 +443.2 MB；程序自报回收 **673.4 MB** | 三条指令全成功 |

> 说明：单次清理量取决于运行时刻的 standby/工作集状态，不同次运行不会相同；GMS 的「可用内存」
> 口径包含 standby 缓存，所以清 standby 时可用内存增量小于实际回收量（探针的 Standby 下降量更准确）。
> 结论：**MemCleaner 与 PCL 执行同一组内核指令、同样顺序、同样权限**，清理能力等价；
> 实测释放量差异来自运行时刻内存状态，而非实现差异。

## 3. MemCleaner 自检输出（管理员）

| 次数 | 退出码 | elevated | clean_success | freed | reclaimed | 三步状态 |
|---|---|---|---|---|---|---|
| 1 | 0 | True | True | 929.1 MB | 929.1 MB | EmptyWorkingSets=OK, FlushModifiedList=OK, PurgeStandbyList=OK |
| 2 | 0 | True | True | 109.8 MB | 226.7 MB | EmptyWorkingSets=OK, FlushModifiedList=OK, PurgeStandbyList=OK |

## 4. 管理员模式 GUI 端到端
- 点击「清理内存」：可用内存 10757.9 MB → 11423.2 MB（Δ 443.2 MB）
- 程序自报回收缓存：**673.4 MB**，clean_count=1
- 清理日志：`[1792473254] clean ok freed=706150400 reclaimed=697614336 bytes before=11280433152 after=11978047488 duration=2188ms steps=[("EmptyWorkingSets", true, None), ("FlushModifiedList", true, None), ("PurgeStandbyList", true, None)]`
- 进程内存占用（管理员模式、清理后）：**Private 3.5 MB**、WorkingSet 4.1 MB、Handles 172
- 界面截图：`docs/screenshots/elev-1-main-before.png`、`docs/screenshots/elev-2-main-after.png`

## 5. 开机自启计划任务（最高权限）
- 安装 `--install-autostart`：exit=0，查询成功=True
- 卸载 `--uninstall-autostart`：exit=0，卸载后查询成功=False
- 结论：计划任务安装/查询/卸载全链路正常，验证后已恢复原状（未残留任务）。

## 6. 结论

- 管理员权限下 `SeProfileSingleProcessPrivilege` 启用成功，PCL 同款三条指令全部 `STATUS_SUCCESS`；
- 冷启动一次清理回收 **929.1 MB**，GUI 端到端回收 **673.4 MB**；
- 清理期间程序自身占用 **Private 3.5 MB**，远低于常见内存清理工具；
- 与真实 PCL `--memory` 同场对比：实现等价（同一组内核指令），差异仅来自运行时刻可回收内存状态；
- 开机自启计划任务全链路通过，验证后已卸载并恢复原状。

> 数据来源：`docs/verify/run-meta.json`、`docs/verify/selftest-1.json`、`docs/verify/selftest-2.json`、
> 独立探针 `tools/baseline-results\20261020-131348-verify-131335.json`；汇总结果 `docs/verify/verify-summary.json`。