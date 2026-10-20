# BaselineProbe —— PCL 2.13.1.1 「内存优化」原理验证探针

C# / .NET 8 控制台程序，用于独立复现并验证 PCL 2.13.1.1「内存优化」的可观察行为（仅行为参考，未复制其代码）：
`SeProfileSingleProcessPrivilege` 启用 + `NtSetSystemInformation(SystemMemoryListInformation=80, &command, 4)`，
command 依次为 `2`（EmptyWorkingSets）→ `3`（FlushModifiedList）→ `4`（PurgeStandbyList）；
以及非管理员时 `--memory` runas 子进程的退出码协议（退出码 = 释放的 KB）。

## 构建 / 运行

```powershell
cd .\probes\BaselineProbe
dotnet build -c Release
.\bin\Release\net8.0\BaselineProbe.exe --help
```

结果 JSON 默认写入 `.\probes\baseline-results\`（每次运行另写一份 `<时间戳>-<tag>.json` 和覆盖 `latest.json`）。

## 常用命令

```powershell
# 1) 普通（标准用户）权限：权限探测 + PCL 三连 + 诊断矩阵 + 本进程工作集裁剪
.\bin\Release\net8.0\BaselineProbe.exe --diag --self-trim --tag nonadmin-full

# 2) 只验证三连中某一条（标准用户下命令 3 会成功，2/4 返回 0xC0000061）
.\bin\Release\net8.0\BaselineProbe.exe --skip-adjust --commands 2,3,4 --repeat 3 --tag nonadmin-seq

# 3) 提权终端（你自己先以管理员身份打开 PowerShell）里跑完整验证
.\bin\Release\net8.0\BaselineProbe.exe --diag --self-trim --tag elevated
#    并模拟 PCL 的 --memory 子进程协议（退出码 = 释放 KB；失败 -1）
.\bin\Release\net8.0\BaselineProbe.exe --pcl-child --tag pcl-child
echo $LASTEXITCODE

# 4) 从「非提权终端」一条命令完成提权子进程全流程（会弹出 1 次 UAC）
.\bin\Release\net8.0\BaselineProbe.exe --runas-child --diag --self-trim --tag uac-parent
```

> `--runas-child` 只触发 1 次 UAC：父进程用 `ProcessStartInfo{UseShellExecute=true, Verb="runas"}` 启动自身
> 的 `--pcl-child`，等待退出码并按 PCL 的方式把退出码当 KB。UAC 被取消时记录 Win32 错误 1223
> (`ERROR_CANCELLED`)，不会死循环重试。
>
> 如果更希望「手动打开管理员 PowerShell 再跑」，使用
> `powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\uac-run-elevated.ps1`
> （脚本会自检提权、运行完整矩阵与 `--pcl-child`、汇总退出码）。

## 选项

| 选项 | 说明 |
|---|---|
| `--commands 2,3,4` | MEMORY_LIST_COMMAND 列表；默认即 PCL 的顺序 |
| `--repeat N` | 整段命令序列重复 N 轮 |
| `--settle-ms N` | 每轮结束后等待 N ms 再测一次 "settled" 可用内存（默认 1000，0=关闭）|
| `--self-trim` | 测试本进程 `SetProcessWorkingSetSize(-1,-1)` / `EmptyWorkingSet`（标准用户可成功，只影响自身工作集）|
| `--diag` | 诊断矩阵：info class 79/80/81/0x7FFFFFFF、payload 长度 0/2/8/16、命令 0/1/2/3/4/5/6/-1 |
| `--privilege-only` | 只输出 token/特权信息（不调用 NtSetSystemInformation）|
| `--pcl-child` | 模拟 PCL `--memory` 子进程：先查 HasAdminRole，再启用特权、执行 2→3→4、按协议退出 |
| `--runas-child` | 以 Verb=runas 启动自身 `--pcl-child` 并回收退出码（1 次 UAC）|
| `--skip-adjust` | 不调用 AdjustTokenPrivileges，观察"没有尝试提权"时的原始 NTSTATUS |
| `--tag` / `--out-dir` | 结果文件标签 / 输出目录 |
| `--help` | 帮助 |

## 关键判读

- `AdjustTokenPrivileges` 对不存在该特权的令牌 **返回 TRUE，但 last error = 1300 (ERROR_NOT_ALL_ASSIGNED)**。
  PCL 的判定是 `!ret || GetLastWin32Error() != 0`，因此会抛
  `获取内存优化权限失败（错误代码：1300）`；探针 JSON 里 `Token.AdjustPclExact.PclWouldThrow=true`。
- `NtSetSystemInformation` 返回的是 **NTSTATUS**（不是 Win32 错误）：
  - `0x00000000` = STATUS_SUCCESS
  - `0xC0000061` = STATUS_PRIVILEGE_NOT_HELD（缺 SeProfileSingleProcessPrivilege，标准用户令牌里根本没有该特权）
  - `0xC0000004` = STATUS_INFO_LENGTH_MISMATCH（payload 长度 < 4 字节）
  - `0xC000000D` = STATUS_INVALID_PARAMETER（命令值 <0 或 >=6）
  - `0xC0000003` = STATUS_INVALID_INFO_CLASS（信息类非法）
- **重要实测（Windows 11 build 26300，标准用户）**：命令 `3`（MemoryFlushModifiedList）**不需要任何特权，返回
  STATUS_SUCCESS 并真实清空 Modified Page List**（45~130 MB → 几 KB，可用物理内存增加数十 MB，耗时 30~334 ms）；
  命令 `2`、`4`、`5`、`0`、`1` 则返回 `0xC0000061`。PCL 因为先做管理员检查，永远走不到这条"免提权"路径。
- `NtQuerySystemInformation(class=80)` 标准用户也能成功，可用于读取 standby/modified/free/zero 页统计。
- 标准用户令牌（UAC 过滤）中 `SeProfileSingleProcessPrivilege` **完全不存在**（不是 Disabled），
  `WindowsPrincipal.IsInRole(Administrator) == false`；实测 list: SeShutdown / SeChangeNotify(Enabled) /
  SeUndock / SeIncreaseWorkingSet / SeTimeZone。
- PCL 的 `ComputerInfo.AvailablePhysicalMemory` == `GlobalMemoryStatusEx.ullAvailPhys`（已在本机逐次对比，
  差值仅为两次读取之间的实时变化）。

## 结果 JSON 主要字段

```
Host / ProcessInfo / Token / MemoryBefore / MemoryAfter
SystemMemoryListBefore/After   # NtQuerySystemInformation(80) 的页统计（字节，按 priority 0..7）
Iterations[].Steps[]           # command / status / delta / elapsed
Diagnostics[]                  # info class / length / command 边界矩阵
SelfTrim                       # 本进程工作集裁剪
PclChild                       # --pcl-child 模式的 before/after/exitCode/内存列表前后
RunAsChild                     # --runas-child 模式的 UAC 子进程退出码 / 错误 / 耗时
```

## 实测结论与完整规格

见 `.\docs\pcl-reverse-engineering.md`；需要在用户授权 UAC 后完成的项目见
`.\tools\uac-test-plan.md`。已保存的非管理员实测结果在 `tools\baseline-results\`。
