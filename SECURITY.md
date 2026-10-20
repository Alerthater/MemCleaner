# 安全政策（Security Policy）

## 支持范围

| 版本 | 支持状态 |
|---|---|
| 1.0.x | ✅ 接受安全报告 |
| < 1.0 | ❌ 不再支持 |

## 报告漏洞

请**不要**在公开 Issue 中披露安全问题。推荐使用 GitHub 私密安全公告：

- https://github.com/Alerthater/MemCleaner/security/advisories/new

报告中请包含：受影响版本、Windows 版本、复现步骤、影响描述（例如是否可导致提权、
任意内存/文件访问、计划任务滥用等）。我们会在收到报告后尽快确认并修复，修复发布后
会在 Release Notes 与 Advisory 中致谢（如你愿意具名）。

## 安全设计说明

MemCleaner 的设计目标是最小攻击面、透明可审计：

1. **无网络能力**：运行时不发起任何网络请求，不内置自动更新，不下载/执行任何远程代码；
2. **无第三方原生代码**：Rust 依赖只有 `windows-sys`（微软官方 Win32 元数据绑定）、
   `serde`/`serde_json`；不加载第三方 DLL，不使用内核驱动或系统服务；
3. **提权仅用于系统级清理**：管理员权限只用于调用公开的 Windows 内存管理接口
   （`NtSetSystemInformation` 等）与安装/卸载自启计划任务；不读取用户文档、不修改系统设置；
4. **计划任务可审计**：自启任务名为 `MemCleaner AutoStart`，只在该用户登录时以最高权限
   启动本程序 `--autostart`；可随时在设置页关闭或 `MemCleaner.exe --uninstall-autostart` 删除；
5. **配置与日志**：配置为本地 JSON（原子写入、加载时范围校验）；日志默认关闭；
6. **单实例**：使用命名互斥体；高完整性实例存在时，普通权限实例会唤醒已有窗口后退出，避免配置竞争；
7. **无持久化后台服务**：不注册 Windows 服务、不写启动项以外的注册表位置。

## 内存清理的固有影响（非安全漏洞）

- `PurgeStandbyList` 会清空系统文件缓存，短时间内读写磁盘可能变慢（机械硬盘更明显）；
- `EmptyWorkingSets` 会把所有进程的工作集移出，程序会按需重新加载；
- 清理不会结束进程、不会丢失未保存数据；但建议避免在大型游戏、编译、文件拷贝过程中清理。

## 下载渠道

请只从本仓库 Releases 页面下载构建产物：
https://github.com/Alerthater/MemCleaner/releases

如二进制哈希与 Release Notes 中公布的不一致，请勿运行并向我们报告。
