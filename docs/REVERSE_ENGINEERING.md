# 互操作性研究记录：PCL「内存优化」行为与 MemCleaner 的实现规格

> 本文是 **MemCleaner** 的设计文档，记录我们如何理解并独立实现「系统级内存清理」这一功能。
> 本文不包含、也不引用 PCL 的源代码或反编译代码文本；只描述可观察行为、公开的 Windows API
> 语义与我们的独立实现。第三方项目的版权与许可边界见 [../NOTICE.md](../NOTICE.md)。

## 1. 背景与范围

- 目标软件：Plain Craft Launcher 2（PCL）2.13.1.1，其「更多 → 百宝箱 → 内存优化」提供系统级内存清理；
- 研究目的：理解该功能的**可观察行为**，以便在 Rust 中独立实现相同效果（互操作性）；
- 研究边界：只涉及 Windows 公开系统调用与行为观察；不复制、不翻译、不再分发 PCL 的实现代码。

## 2. 研究方法

1. **行为观察**：在测试机上运行 PCL 的「内存优化」与 `Plain Craft Launcher 2.exe --memory`，
   观察前后系统可用内存、进程工作集、待机/已修改页面列表的变化；
2. **逆向分析**：对发布版程序做静态分析，确认其调用的 Windows API 与参数（用于确定互操作性
   所需的调用序列）。分析结果仅用于确认 API 序列，不进入本仓库；
3. **独立探针复现**：用 C# 编写独立探针 `probes/BaselineProbe`，按推断出的序列调用 API，
   读取 `MEMORYSTATUSEX` 与 `NtQuerySystemInformation(SystemMemoryListInformation)` 验证效果；
4. **同场对照**：在同一台机器、同一时段分别运行 PCL `--memory` 与 MemCleaner，对比清理效果。

## 3. 行为结论

### 3.1 调用序列（PCL 的可观察行为）

1. 要求当前进程具备管理员权限（UAC 提升）；
2. 启用 `SeProfileSingleProcessPrivilege`（通过 `LookupPrivilegeValue` + `AdjustTokenPrivileges`）；
3. 依次调用 `NtSetSystemInformation`，`SystemInformationClass = SystemMemoryListInformation = 80`，
   参数为 4 字节的 `MEMORY_LIST_COMMAND` 值：

| 值 | 命令 | 作用 | 是否需要 `SeProfileSingleProcessPrivilege` |
|---|---|---|---|
| 2 | `MemoryEmptyWorkingSets` | 清空所有进程工作集 | 是 |
| 3 | `MemoryFlushModifiedList` | 刷新已修改页面列表 | 实测在 Win11 26300 标准用户下也可成功，但 PCL 因先检查管理员权限而不会走到该分支 |
| 4 | `MemoryPurgeStandbyList` | 清除待机列表 | 是 |
| 5 | `MemoryPurgeLowPriorityStandbyList` | 仅清除低优先级待机列表（PCL 未使用；Mem Reduct 同类功能） | 是 |

> 枚举取值依据：Windows DDK/phnt 公开头文件中的 `MEMORY_LIST_COMMAND`；亦可通过
> `NtQuerySystemInformation(80)` 读取 `SYSTEM_MEMORY_LIST_INFORMATION` 交叉验证。

4. 统计口径：清理前后读取 `MEMORYSTATUSEX.ullAvailPhys`，差值即「可用内存变化量」；
5. 失败处理：任一指令返回非 0（NTSTATUS）即中止后续指令。

### 3.2 非管理员时的进程间协议

- PCL 以 `ShellExecute` + `Verb="runas"` 启动自身并附加 `--memory` 参数；
- 被提升的子进程执行上述清理，然后以**退出码**回传结果：`退出码 = round(Δ可用内存 / 1024)`（KB），
  `-1` 表示失败、`0` 表示无收益；父进程等待子进程退出后读取退出码。

MemCleaner 采用更直接的方案：普通权限下由 UI 弹应用内确认框，确认后整体以管理员身份重启并
立即清理；同时提供「最高权限计划任务」实现开机静默启动与自动清理（免每次 UAC）。


### 3.4 进阶清理项与所需特权（v1.1.0 补充）

- `SetSystemFileCacheSize(-1, -1, 0)`（收缩系统文件缓存，Mem Reduct 同类功能）需要
  **`SeIncreaseQuotaPrivilege`**；v1.0.0 缺少启用步骤导致整次清理失败，v1.1.0 在调用前显式启用，
  管理员实测返回成功（`TrimFileCache` 步骤 `STATUS_SUCCESS`）。
- `MemoryPurgeLowPriorityStandbyList`（命令 5）需要 `SeProfileSingleProcessPrivilege`，
  管理员权限下实测成功；PCL 未使用该命令，属 MemCleaner 的进阶可选项。

### 3.3 测量陷阱

- **可用内存包含待机缓存**：`ullAvailPhys` 把 standby 页算作可用，所以只清除待机列表时可用内存
  可能几乎不变。要衡量真实清理量应读取 `NtQuerySystemInformation(80)` 的 standby/modified 页数变化；
- **单次清理量不可复现**：取决于运行时刻的工作集与 standby 状态，连续两次清理第二次必然更少；
- **GMS 采样有抖动**：清理期间系统仍在分配/释放内存，建议同时记录程序自报的回收量与前后列表量。

## 4. MemCleaner 的实现映射

| 行为 | 源文件 |
|---|---|
| `GlobalMemoryStatusEx` / `NtSetSystemInformation(80)` / `NtQuerySystemInformation(80)` / `SetSystemFileCacheSize` | `src/sys/memory.rs` |
| 管理员判定、特权启用 | `src/sys/privilege.rs` |
| 测量 → 特权 → 2/3/4 → 再测量的清理引擎 | `src/engine.rs` |
| 配置到清理项的映射、自动清理判定 | `src/engine.rs` |
| UAC 提权重启、单实例、计划任务、托盘、自绘 UI | `src/platform/`、`src/ui/` |
| 命令行自检（JSON 输出，含每步 NTSTATUS） | `src/selftest.rs` |

## 5. 实测数据（管理员权限）

测试环境：Windows 11 Pro build 26300，16 GB 内存。

| 指令 | NTSTATUS | 可用内存瞬时增量 | 耗时 |
|---|---|---|---|
| 2 `MemoryEmptyWorkingSets` | `0x00000000` | +1809.1 MB | 976.9 ms |
| 3 `MemoryFlushModifiedList` | `0x00000000` | +3152.7 MB | 4283.1 ms |
| 4 `MemoryPurgeStandbyList` | `0x00000000` | -357.3 MB | 1822.3 ms |

- 系统内存列表：Standby **4953.6 MB → 63.6 MB**；Modified **24.9 MB → 9.7 MB**；
- 冷启动一次 `--self-test` 回收 **929.1 MB**（1927 ms）；
- 同场 PCL `--memory`：381.4 MB / 349.4 MB（退出码 390508 / 357752 KB）。

> 两条指令之间的"可用内存瞬时增量"会因系统并发活动波动，不是严格的累计值；结论以三条指令均
> `STATUS_SUCCESS`、Standby 大幅下降为准。完整报告：[VERIFICATION.md](VERIFICATION.md)。

## 6. 如何复现

```powershell
# 1) 构建独立探针（需要 .NET 8 SDK）
cd probes/BaselineProbe
dotnet build -c Release

# 2) 在「管理员」终端中运行完整矩阵（会真实清理一次内存）
.\bin\Release\net8.0\BaselineProbe.exe --diag --self-trim

# 3) 或只验证 PCL 子进程协议等价形式（退出码 = 释放 KB）
.\bin\Release\net8.0\BaselineProbe.exe --pcl-child
```

MemCleaner 自身的等价验证：

```powershell
# 以管理员身份运行，会执行真实清理并输出每步 NTSTATUS 的 JSON
.\MemCleaner.exe --self-test --out selftest.json
```

## 7. 适用条件与限制

- 仅支持 Windows x64（Windows 10 1809+ / Windows 11），原生 ARM64 未验证；
- 这些调用属于原生 API，语义在不同 Windows 版本上可能变化；本仓库在 Win11 26300 上实测；
- 标准用户下 `FlushModifiedList` 可能成功属于系统行为，不构成安全绕过；
- 本项目只实现「内存清理」这一功能，不涉及 PCL 的启动器/下载/实例管理等其它功能。

## 8. 参考来源

- PCL（Plain Craft Launcher）：https://github.com/Meloong-Git/PCL —— 行为参考；其许可为自定义的
  《PCL 分发有限许可》+《存储库合理使用指南》；
- phnt（MIT）：`MEMORY_LIST_COMMAND` / `SYSTEM_MEMORY_LIST_INFORMATION` 等原生常量；
- Microsoft Learn：`GlobalMemoryStatusEx`、`SetSystemFileCacheSize`、`AdjustTokenPrivileges`、
  DWM/GDI/Shell 等公开文档；
- Mem Reduct（GPL-3.0）：同类功能划分参考（未复制代码）。

完整的版权与许可边界见 [../NOTICE.md](../NOTICE.md) 与 [../LEGAL.md](../LEGAL.md)。
