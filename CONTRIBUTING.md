# 贡献指南（Contributing）

感谢你对 MemCleaner 的兴趣！欢迎提交 Issue 与 Pull Request。

## 提交 Issue

- Bug 报告请注明：Windows 版本、DPI/缩放、MemCleaner 版本、是否管理员模式、复现步骤、
  期望/实际行为；不要包含个人敏感信息；
- 安全漏洞**不要**公开提交，请按 [SECURITY.md](SECURITY.md) 使用私密安全公告。

## 提交 Pull Request

1. Fork 本仓库，从 `main` 创建分支；
2. 保持代码风格：`cargo fmt`；提交前跑通 `cargo test` 与 `cargo build --release`；
3. PR 描述中说明动机、实现方式与测试情况（含截图更佳）；
4. 一次 PR 聚焦一件事，避免无关重排/格式化噪声。

## 贡献许可

提交 PR 即表示你同意你的贡献以本项目的 [MIT License](LICENSE) 授权。

## 内容合规要求（重要）

由于本项目涉及对第三方软件行为的参考，提交内容必须遵守：

1. **不要**提交任何来自 PCL、Mem Reduct 或其他第三方项目的源代码、反编译代码、
   资源文件、图标、字体或二进制；
2. **不要**提交用于绕过授权/验证的代码或工具；
3. 新增功能如参考了其它项目的行为或文档，请在 [NOTICE.md](NOTICE.md) 中补充来源与许可说明；
4. 仅使用官方渠道/公开文档获取的 API 信息，不提交泄露的私有资料。

## 开发提示

- 渲染层是纯软件光栅化（`src/ui/raster.rs` + `src/ui/design.rs`），新增控件请保持零额外依赖；
- 所有系统调用封装在 `src/sys/` 与 `src/platform/`，业务层不得直接 unsafe 调 Win32；
- 提交前请确认：`cargo test` 全绿、无新增 `unsafe` 泄漏到业务层、不引入网络能力；
- 性能红线：空闲 Private Usage 不应超过 6 MB，单帧渲染 < 10 ms（125% DPI）。
