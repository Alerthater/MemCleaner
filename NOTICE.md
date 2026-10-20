# 来源声明与第三方致谢（NOTICE）

本项目 **MemCleaner** 是独立开发的开源软件，源代码以 [MIT License](LICENSE) 发布。

在设计与实现过程中，本项目参考了以下公开项目/文档的**功能行为与系统接口说明**。
除下方明确列出的情况外，本仓库不包含、不分发上述项目的任何源代码、反编译产物、
二进制文件、图片或字体资源。

---

## 1. Plain Craft Launcher（PCL）

| 项目 | 说明 |
|---|---|
| 名称 | Plain Craft Launcher 2（PCL） |
| 作者 | 龙腾猫跃（LTCat） |
| 项目地址 | https://github.com/Meloong-Git/PCL |
| 官方网站 | https://meloong.com |
| 参考版本 | PCL 2.13.1.1（本研究开始时用户提供的发布版） |
| 许可 | 《PCL 分发有限许可》+《PCL 存储库合理使用指南》（自定义许可，见其仓库 `LICENCE` 文件与 https://shimo.im/docs/rGrd8pY8xWkt6ryW ） |

**参考内容与方式：**

- 本项目参考了 PCL「内存优化」功能的**可观察行为**，以实现兼容的系统级内存清理能力。
- 为确认互操作性所必需的系统调用序列（清理动作、顺序、所需权限），对 PCL 2.13.1.1
  发布版程序进行了黑盒测试与逆向分析；该功能的源码并未包含在 PCL 的开源仓库中。
- 本项目**没有**复制、改写、翻译或再分发 PCL 的源代码、反编译代码、资源文件或二进制文件；
  本仓库内也不包含任何 PCL 反编译产物。Rust 实现是从零编写的独立实现。
- 本项目**不是** PCL 的分支、修改版、衍生作品或官方版本，与 PCL 作者无隶属或合作关系；
  项目名称「MemCleaner」与界面设计均为原创，未使用 PCL 名称或品牌。

**署名（依据 PCL 合理使用指南「轻度使用」要求）：**

> 感谢 **龙腾猫跃（LTCat）** 与 **Plain Craft Launcher (PCL)** 项目。
> PCL 是一个优秀的 Minecraft 启动器，其「内存优化」功能为本项目的设计提供了行为参考：
> https://github.com/Meloong-Git/PCL

如 PCL 作者对本项目的署名方式或引用范围有异议，请联系我们，我们会及时调整或移除相关内容。

---

## 2. Mem Reduct

| 项目 | 说明 |
|---|---|
| 名称 | Mem Reduct |
| 作者 | Henry++（2011–2026） |
| 项目地址 | https://github.com/henrypp/memreduct |
| 许可 | GNU General Public License v3.0（GPL-3.0） |
| 参考内容 | 同类软件的功能划分与用户预期（如「系统文件缓存清理」「低优先级待机列表清理」等命名与行为概念） |

**声明：** 本项目**没有**复制或改编 Mem Reduct 的任何源代码（包括其 `main.c` 等文件），
也未链接或分发其二进制。MemCleaner 的 Rust 代码为独立实现；本项目不是 Mem Reduct 的
衍生作品。Mem Reduct 的 GPL-3.0 许可仅适用于 Mem Reduct 自身。

---

## 3. phnt（Process Hacker / System Informer Native API 头文件）

| 项目 | 说明 |
|---|---|
| 项目地址 | https://github.com/winsiderss/phnt |
| 许可 | MIT License |
| 参考内容 | 核对 `NtSetSystemInformation` 的 `SystemMemoryListInformation`（class = 80）与 `MEMORY_LIST_COMMAND` 枚举取值（2 = EmptyWorkingSets、3 = FlushModifiedList、4 = PurgeStandbyList、5 = PurgeLowPriorityStandbyList） |

**声明：** 仅用于交叉验证公开的 Windows 原生 API 常量定义，未复制头文件代码进本仓库。
相关系统调用说明同样可参阅 Microsoft 官方文档与 Windows SDK/DDK 公开资料。

---

## 4. Microsoft Windows 文档与系统组件

本项目实现依赖 Windows 公开系统接口，行为说明参考 Microsoft Learn 官方文档，包括但不限于：

- `GlobalMemoryStatusEx`、`SetSystemFileCacheSize`、`NtSetSystemInformation`（原生 API）
- `SeProfileSingleProcessPrivilege` / `SeIncreaseQuotaPrivilege` 特权模型
- DWM（`DwmSetWindowAttribute`、`DwmExtendFrameIntoClientArea`）、GDI、Shell_NotifyIcon、Task Scheduler 等

Windows 与相关 API 的版权归 Microsoft Corporation 所有；本项目仅调用公开系统接口，不重分发任何 Microsoft 组件。
界面使用系统自带字体（Segoe UI Variable、Microsoft YaHei UI、Segoe MDL2 Assets），不随附字体文件。

---

## 5. Rust 依赖库（crates.io）

本项目的二进制依赖均为宽松许可（MIT / Apache-2.0）的开源库，通过 crates.io 在构建时获取，
本仓库不内嵌其源代码：

| 依赖 | 版本 | 许可 |
|---|---|---|
| [`windows-sys`](https://crates.io/crates/windows-sys) | 0.61.x | MIT OR Apache-2.0 |
| [`windows-link`](https://crates.io/crates/windows-link)（windows-sys 的传递依赖） | 0.2.x | MIT OR Apache-2.0 |
| [`serde`](https://crates.io/crates/serde) / [`serde_derive`](https://crates.io/crates/serde_derive) | 1.x | MIT OR Apache-2.0 |
| [`serde_json`](https://crates.io/crates/serde_json) | 1.x | MIT OR Apache-2.0 |
| [`itoa`](https://crates.io/crates/itoa)、[`ryu`](https://crates.io/crates/ryu)（serde_json 的传递依赖） | 1.x | MIT OR Apache-2.0 |

完整依赖树见 `Cargo.lock`。构建工具（Rust toolchain、MSVC、Windows SDK）不属于本项目分发内容。

---

## 6. 本项目原创内容

- `src/` 下的全部 Rust 源代码、构建配置与文档：Copyright (c) 2026 Alerthater，MIT License；
- 界面设计、应用图标（由代码程序化绘制）与截图：本项目原创；
- `probes/BaselineProbe`（独立验证探针）：本项目原创，MIT License；
- 文档中的实测数据来自本项目在测试机上的真实测量结果。

---

## 7. 商标声明

「Plain Craft Launcher」「PCL」为龙腾猫跃的标识；「Mem Reduct」为 Henry++ 的标识；
「Windows」「Microsoft」为 Microsoft Corporation 的商标；「Rust」为 Rust Foundation 的商标。
本项目使用这些名称仅用于事实性说明与致谢，不表示任何隶属、赞助或官方认可关系。
