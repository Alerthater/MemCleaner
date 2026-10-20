# 更新日志（Changelog）

本项目遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## v1.0.0 — 2026-10-20

首个公开版本。

### 新增

- 系统级内存清理：与 PCL 2.13.1.1「内存优化」相同的三条内核指令
  （`MemoryEmptyWorkingSets` / `MemoryFlushModifiedList` / `MemoryPurgeStandbyList`），
  自动启用 `SeProfileSingleProcessPrivilege`，需要管理员权限；
- 进阶清理项：低优先级待机列表、系统文件缓存收缩（默认关闭）；
- 极简 Win11/Mac 风无边框 UI：DWM 圆角+阴影、软件光栅化抗锯齿圆环、60fps 按需动画、
  深色/浅色跟随系统、应用内确认框、托盘菜单、单实例；
- 自动清理：阈值（默认 85%）/固定间隔 + 冷却时间；
- 开机自启：以最高权限计划任务实现登录静默启动到托盘，自动清理免 UAC；
- `--self-test` 自检模式（JSON 输出）、`--install-autostart` / `--uninstall-autostart`；
- 配置原子写入与范围校验；单实例在「管理员实例常驻 + 普通权限再次启动」场景下正确唤醒。

### 实测

- 常驻 Private 3.46–3.48 MB，Release 单文件 404,992 字节；
- 管理员模式冷启动一次清理回收 929.1 MB（同场 PCL 381.4 MB）；
- QA 自动化 111/111 断言通过，`cargo test` 20/20；
- 管理员权限验证（独立探针 + 真实 PCL 对比 + 计划任务）全部通过。

### 说明

- 内存清理会清空待机列表/工作集，短时磁盘读性能下降属预期行为；
- 项目为独立实现，参考来源与许可边界见 [NOTICE.md](NOTICE.md)。
