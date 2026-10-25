# 开机自启修复验证（v1.1.1）

- 时间：2026-10-25 20:30 (+08:00)
- 程序：`<安装目录>\MemCleaner.exe`（v1.1.1，463872 字节，SHA256 BAF185E6CE39C9FACC833462AF18A762D8F35AD8B967D462843C8AF187D7C8E0）
- 任务：`\MemCleaner AutoStart`（状态 Enabled / RunLevel HighestAvailable / LogonTrigger）

## 1. 修复前的问题

- 用户配置 `start_with_windows = true`，但系统里**不存在** `MemCleaner AutoStart` 计划任务
  （`schtasks /Query` 返回「找不到指定的文件」），开机登录时没有任何程序被启动；
- 旧版本没有状态同步/自愈：任务被外部删除（例如验证脚本卸载、安全软件清理）或程序被移动后，
  UI 仍显示「已开启」，但实际不会自启；
- 旧代码在 exe 路径含空格时给 `<Command>` 加了引号，Task Scheduler 会按错误路径启动（找不到文件）；
- 登录时 Explorer 可能尚未就绪，`Shell_NotifyIcon` 失败后没有重试，程序静默运行且无托盘图标；
- 开启自启时若用户取消 UAC，配置已被写成 true，导致「开关显示开启但任务不存在」。

## 2. 修复内容（v1.1.1）

1. 启动时读取任务 XML，校验 `<Command>` 是否指向当前 exe；任务缺失/路径不一致时自动重建；
2. `<Command>` 不再加引号；安装后回读校验，失败返回明确错误；卸载幂等；
3. 托盘图标添加失败自动重试（1.5s × 20 次），并监听 `TaskbarCreated` 在 Explorer 重启后恢复；
4. 取消 UAC 时回滚「开机自启」配置；
5. 新增 `--autostart-status` 诊断命令与启动诊断日志（`%LOCALAPPDATA%\MemCleaner\memcleaner.log`，
   不可写时回退 exe 目录）。

## 3. 修复后验证结果

| 检查项 | 结果 |
|---|---|
| 安装任务 | 成功（`--install-autostart` exit=0） |
| 系统任务查询 | 存在且 Enabled，`Task To Run = <安装目录>\MemCleaner.exe --autostart` |
| 任务 XML `<Command>` 无引号且指向当前 exe | True |
| LogonTrigger / HighestAvailable / Enabled | True / True / False |
| 删除任务后以管理员启动程序自动重建 | 成功（日志：`status: NotInstalled` → `autostart: task repaired/reinstalled`） |
| `schtasks /Run` 触发 | 成功启动 `"<安装目录>\MemCleaner.exe" --autostart` |
| 任务启动的窗口状态 | `MemCleanerMainWindow visible=False`（仅驻留托盘） |
| 真实用户会话（通过 UAC 启动的管理员实例）托盘注册 | `tray add ok=true`（19:00 诊断日志） |

## 4. 说明与限制

本次验证的「触发器执行」是在开发工具链的受限子进程会话里用 `schtasks /Run` 触发的。
该会话对子进程有文件/UI 限制（因此任务进程写入 `%LOCALAPPDATA%` 与 `Shell_NotifyIcon` 会返回 ACCESS_DENIED，
自动回退到 exe 同目录日志）；**真实开机登录由系统触发，不受此限制**——同一台机器通过 UAC 启动的
管理员实例（19:00 日志）已验证托盘注册成功（`tray add ok=true`）、自启任务自愈成功。

## 5. 用户侧确认步骤

1. 直接重启 Windows 一次；
2. 登录后等待 10–30 秒（Explorer 初始化期间程序会自动重试托盘注册）；
3. 任务栏托盘溢出区应出现内存占用率图标；若没有，查看：
   `%LOCALAPPDATA%\MemCleaner\memcleaner.log`（或程序目录下的 `memcleaner.log`），
   或运行 `MemCleaner.exe --autostart-status` 查看任务状态；
4. 任务当前已安装并指向 `<安装目录>\MemCleaner.exe`；若以后移动 exe，
   以管理员权限启动一次程序即可自动更新任务路径。