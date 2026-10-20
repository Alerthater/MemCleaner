# 隐私政策（Privacy Policy）

**适用软件：** MemCleaner（Windows 桌面程序）
**生效日期：** 2026-10-20
**适用范围：** 本仓库发布的所有版本（当前 1.0.x）

MemCleaner 被设计为**完全离线**的本地工具。请在使用前阅读本政策。

---

## 1. 我们收集哪些数据？

**我们不收集任何数据。**

- 无网络请求：程序运行期间不发起任何 HTTP/HTTPS 连接，不包含遥测、崩溃上报、自动更新、
  广告或统计 SDK；
- 无账号体系：无需注册、登录，不采集设备标识、硬件指纹、IP 或使用行为；
- 无云端同步：所有设置只保存在本机。

## 2. 本机产生的数据

| 数据 | 位置 | 用途 | 是否必须 |
|---|---|---|---|
| 配置文件 `config.json` | `%LOCALAPPDATA%\MemCleaner\config.json` | 保存界面主题、自动清理开关/阈值、清理项等设置；原子写入 | 是（可删除，删除后回到默认设置） |
| 诊断日志 `memcleaner.log` | `%LOCALAPPDATA%\MemCleaner\memcleaner.log` | 仅在用户主动设置环境变量 `MEMCLEANER_LOG=1` 时写入，用于排查问题 | 否（默认不写） |
| 开机自启计划任务 | Windows 计划任务 `MemCleaner AutoStart` | 登录时以管理员权限静默启动到托盘，使自动清理免 UAC | 否（可选，可随时删除） |

以上数据均不上传、不共享给任何第三方。


### 自定义皮肤（v1.1.0）

仅当你主动在设置页选择一张本地图片作为皮肤时，程序才会在本机读取该图片文件（通过系统 GDI+
解码到内存并缩放到窗口尺寸），不会复制、移动、上传该文件；配置文件中仅保存图片的本地路径。
点击「清除自定义皮肤」后，程序会立即释放解码缓存并恢复默认背景。

## 3. 管理员权限与系统操作

内存清理（清空工作集 / 刷新已修改页面 / 清除待机列表）需要管理员权限，程序会：

1. 在需要系统级清理时请求一次 UAC 提权（或通过用户自行启用的计划任务以管理员权限运行）；
2. 调用 Windows 公开接口进行内存清理，**不读取、不修改、不上传用户的个人文件**；
3. 不会结束任何进程、不修改注册表中的系统项、不安装内核驱动或系统服务。

## 4. 数据删除

- 删除配置：删除 `%LOCALAPPDATA%\MemCleaner\` 目录即可；
- 删除自启：设置页关闭「开机自启」，或运行 `MemCleaner.exe --uninstall-autostart`；
- 卸载程序：本项目为便携式软件，删除 `MemCleaner.exe` 即可。

## 5. 第三方组件与网络

程序依赖的 Windows 系统组件与 Rust 依赖库见 [NOTICE.md](NOTICE.md)。依赖库仅在构建时从
crates.io 获取；运行时不联网。界面使用系统自带字体，不内置字体文件。

## 6. 未成年人

本程序不收集任何个人信息，不针对未成年人收集数据。

## 7. 政策变更

若未来引入任何联网或数据收集功能（目前没有此计划），我们会在本文件中显著说明并在新版本
发布说明中告知。继续使用新版本即表示接受更新后的政策。

## 8. 联系方式

请通过本仓库的 GitHub Issues 反馈：https://github.com/Alerthater/MemCleaner/issues
（请不要在公开 Issue 中提交敏感信息。）

---

### English Summary

MemCleaner is a fully offline local tool. It does **not** collect, transmit, or share any
data: no telemetry, no crash reporting, no auto-update, no network requests at runtime.
A settings file is stored at `%LOCALAPPDATA%\MemCleaner\config.json`; an optional log file
is written only when `MEMCLEANER_LOG=1` is set. Administrator rights are used solely to call
documented Windows memory-management APIs, never to read or modify user documents. The only
optional system change is a per-user Task Scheduler entry (`MemCleaner AutoStart`) used for
silent auto-start; it can be removed at any time from the settings page or via
`MemCleaner.exe --uninstall-autostart`.
