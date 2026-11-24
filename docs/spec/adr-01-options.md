# ADR-01 Windows 桌面技术选型

2026-11-18，用户确认采用 **Tauri 2 + Vue 3**（方案 B）。Vue 和 TypeScript 负责界面，Rust 负责 Markdown 文件、SQLite 状态和本地任务。首发 Windows，后续扩展 macOS。功能范围及数据职责见[开发规格](../../SPECIFICATION.md)。

| 选项 | 候选组成 | 适合优先考虑 | 主要取舍 |
|---|---|---|---|
| **A · C# 原生桌面** | .NET Framework 4.8.1、WinForms、Windows 系统 SQLite、Visual Studio 2022 | Windows 单人试用、少引入依赖、尽快验证快捷记录 | 现有工具可编译，托盘和原生输入控件有现成支持。SQLite 的 C API 绑定及高度定制的界面需维护 C# 代码；个人试用可考虑便携包，正式安装包另行验证 |
| **B · Tauri 2 + Vue 3** | Vue 3 和 TypeScript 界面、Rust 应用服务、WebView2、Rust 侧 SQLite 绑定 | 希望用 Vue 做界面、复用系统 WebView2、接受 Rust | 支持托盘、快捷键插件和 Windows 打包。工作区与文件操作放在 Rust 侧，需要同时维护前端和 Rust 工具链 |
| **C · Electron + Vue 3** | Vue 3 和 TypeScript 界面、Node.js 主进程、Chromium、主进程 SQLite | 希望主进程和界面都用 TypeScript、统一渲染环境 | 核心提供托盘和全局快捷键。随应用分发 Chromium 和 Node.js，需要为常驻内存与包体做实测。SQLite 绑定按选定 Electron 版本验证，不能拿本机 Node.js 版本代替 |

平台依据分别见 [Microsoft 的 .NET Framework 支持说明](https://learn.microsoft.com/en-us/dotnet/framework/install/versions-and-dependencies)、[Windows 系统 SQLite](https://learn.microsoft.com/en-us/dotnet/standard/data/sqlite/custom-versions)、[Tauri Windows 前置条件](https://v2.tauri.app/start/prerequisites/)、[Tauri 快捷键](https://v2.tauri.app/plugin/global-shortcut/)、[Tauri 打包](https://v2.tauri.app/distribute/windows-installer/)、[Electron 进程模型](https://www.electronjs.org/docs/latest/tutorial/process-model)、[Electron 快捷键](https://www.electronjs.org/docs/latest/api/global-shortcut)和[Electron 托盘](https://www.electronjs.org/docs/latest/api/tray)。

## 本机证据

- Windows 11 x64，构建号 `26300.9974`。Visual Studio 2022 `17.14.36221.1` 和 Roslyn `4.14.0` 已找到，现有编译器编译并运行了 WinForms 探针。探针创建窗体、中文文本控件和托盘对象，注册并释放测试快捷键。尚未实际用中文输入法组词或验证托盘菜单。
- Windows 自带 SQLite 实测为 `3.53.4`。在内存库中验证了单字、双字和三字中文子串，以及 FTS5 trigram 的三字查询。上述结果覆盖能力探测，尚无实际工作区搜索性能数据。
- 临时文件发布探针中，`File.Move` 在目标已有内容时返回冲突，目标字节保留。此结果只支持新建文件的初步判断。已有 Wiki 的并发编辑和撤销，仍需在所选技术栈下用真实编辑器验证。
- 选型时，本机已有 Node.js `22.21.1`、Rust/Cargo `1.92.0`、MSVC x64 工具和已注册的 WebView2 Runtime `157.0.4341.46`。

## 常驻体验

用户希望应用长时间在后台低占用运行，随时唤起即可打字，并能继续加功能。若目标限于 Windows，建议优先验证 A。WinForms 可用原生控件实现托盘和输入窗，省去 WebView 初始化。B、C 也要实测快记窗首次可输入的时间，常驻占用按应用及关联进程合计。[WebView2 进程模型](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-model)、[Electron 进程模型](https://www.electronjs.org/docs/latest/tutorial/process-model)

三种路线都采用同样的窗口与任务管理方式。托盘和全局快捷键常驻，主窗口按需加载，关闭时释放它独占的界面资源。索引、抓取和模型按任务运行，空闲时等待事件，输入不等待这些任务。快捷窗需要预先加载哪些资源，由唤出耗时与常驻占用的实测决定。扩展模型与记忆服务沿用现有接入契约。[安全边界与性能基线](../../SPECIFICATION.md#5-安全边界与性能基线)

## 后续扩展到 macOS

所选方案以 Windows 为首发平台，并为后续用同一项目交付 macOS 做准备。macOS 的版本范围、机型和交付时间待定。

- **A · WinForms**。可复用 Markdown、数据契约，以及能用跨平台目标构建的业务代码。macOS 需要另做窗口和系统集成，并换用适合 macOS 的 SQLite 接入。[Microsoft 移植说明](https://learn.microsoft.com/en-us/dotnet/core/porting/framework-overview)
- **B · Tauri + Vue**。两端可共用大部分 Vue 界面和 Rust 应用服务。Windows 使用 WebView2，macOS 使用 WKWebView。框架提供托盘和两端全局快捷键支持，窗口激活、焦点和中文输入仍需实机验证。[Tauri 进程模型](https://v2.tauri.app/concept/process-model/)、[托盘](https://v2.tauri.app/learn/system-tray/)、[全局快捷键](https://v2.tauri.app/plugin/global-shortcut/)
- **C · Electron + Vue**。两端可复用 Vue、TypeScript 主进程代码和 Chromium 渲染能力。随应用分发 Chromium 和 Node.js，常驻 CPU、内存和首次唤出耗时需要实测。SQLite 原生绑定也需按目标平台和架构验证。[Electron 框架说明](https://www.electronjs.org/docs/latest/)

Markdown、任务与分歧规则、状态库迁移和服务契约共用。窗口和快捷键、系统凭据、路径及安全写入的差异集中在应用调用系统的部分。若采用 FTS5，两端用相同样本检验中文查询，并明确 SQLite 版本与编译选项。[SQLite 编译选项](https://www.sqlite.org/compile.html)

建议 Windows 快记闭环完成后，尽早在可用的 Mac 上跑一次菜单栏常驻、快捷键从其他应用唤出、直接中文组词、离线保存和休眠恢复。用相同计时口径记录输入就绪时间和常驻占用。正式通过独立下载分发 macOS 版时，再验证安装包、签名和公证。[Apple 分发说明](https://developer.apple.com/documentation/xcode/packaging-mac-software-for-distribution)

## 决策边界

桌面路线已确定，先验证工具链、SQLite 恢复与落盘顺序，再验证快捷窗焦点、中文 IME、托盘退出和交付方式。依赖清单及锁定版本随对应工作包记录。模型协议、OV 和第二种记忆服务分别归入 ADR-02 和 ADR-03。

此前起草的 C# 候选实验草稿（`experiments/adr-01-winforms/`，未编译、未完成）已于 2026-11-24 经确认清理。

## 开发工程依赖

桌面工程使用 Tauri `2.12.6`、Tauri JS API `2.12.5`、Vue `3.5.45`。开发工具包括 Tauri CLI `2.12.5`、`tauri-build` `2.7.5`、Vite `8.3.8`、Vite Vue 插件 `6.0.11`、TypeScript `5.9.3` 和 `vue-tsc` `3.3.14`。直接依赖分别声明在 [package.json](../../package.json) 和 [Cargo.toml](../../src-tauri/Cargo.toml)。[Tauri 的 Vite 配置](https://v2.tauri.app/start/frontend/vite/)

用户确认本地保存采用 `rusqlite 0.43.0` 的 `bundled` 构建和 `sha2 0.11.1`，并在 P2 显式声明已锁定在 `Cargo.lock` 中的 `serde 1.0.229`（`derive` 特性）用于强类型 IPC DTO。SQLite 与应用一同构建，SHA-256 直接校验落盘原文字节。Windows 首版实测打包后的 SQLite 能力，macOS 采用同一锁定依赖后另做实机核对。[rusqlite 版本说明](https://docs.rs/crate/rusqlite/0.43.0)

Windows 托盘启用 Tauri 已锁定的 `tray-icon` 功能。全局热键使用 Windows `RegisterHotKey`，独立消息线程在无窗口时等候事件，并使用 `MOD_NOREPEAT` 避免按住热键重复唤出。默认键位按 DESIGN-01 采用 Win+Alt+N，另提供两个可选组合处理系统或应用占用。[Tauri 托盘](https://v2.tauri.app/learn/system-tray/) · [Windows 热键](https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-registerhotkey)

Windows 新记录使用目标目录内的临时文件，刷新完成后用 `SetFileInformationByHandle` 发布，禁止替换同名目标。发布后持有回读核对过的文件句柄，直到 SQLite 提交。macOS 安全发布方式待对应实现和验证。[Windows 文件重命名语义](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info)

### 平台抽象与状态库 Schema v4

- **平台边界收敛**：在 `src-tauri/src/platform/` 下收口 `HttpTransport` trait、系统凭据读写（Windows `CredReadW` / `CredWriteW`）、外部打开与资源管理器定位（`ShellExecuteW` / `explorer.exe`）、前台焦点恢复（`ForegroundWindow`）和文件共享锁错误判定。`dto.rs` 与 `model.rs` 不再含平台专有代码或 `unsafe` 块，为后续 WP-06/07 与 macOS 扩展留清接口。
- **增量索引与 FTS5 混合检索**：Schema v4 为 `search_documents` 增加 `mtime_ns` 与 `file_size`，并建立外部内容表 `search_documents_fts`（`fts5(..., tokenize='trigram')`）及同步触发器。重建扫描采用 `(mtime_ns, file_size, captured_at)` 加 2 秒 racy-clean 窗口跳过未变更文件的磁盘读取与 SHA-256 哈希计算；检索对 ≥ 3 个 Unicode 字符的查询先经 FTS5 trigram 预筛再做字面子串确认，1–2 字符保留 `instr(lower(...))` 全量匹配，确保单/双汉字 100% 召回。首次查询扫描工作区，后续查询通过 `ensure_index_fresh()` 增量纳入应用内保存。
- **分歧关系规范化**：Schema v4 将原 `m2_disagreements` 单列 JSON 迁移至规范化的 `disagreements`、`disagreement_members`（建立 `claim_id`、`(source_id, revision)`、`(file_identity, revision)` 与 `evidence_id` 索引）和 `disagreement_history` 表；问答相关分歧回查改走索引定位，仅对命中分歧按需校验磁盘证据。旧版 v1–v3 工作区在打开时于单事务内自动无损升级至 v4。[SQLite FTS5 trigram](https://sqlite.org/fts5.html#the_trigram_tokenizer)

### 2026-11-24 v1.0 务实减负修订（R1 起实施）

经 2026-11-24 全项目检视与产品决策确认，上述 M1/M2/P2 阶段沉淀的部分防御设计对单人桌面工具过重，自 v1.0 起按以下务实原则精简（保留 Tauri 2 + Vue 3 + Rust + 内置 SQLite FTS5 技术路线不变）：

- **文件系统与配置减负**：移除常驻父目录链句柄锁、移除对 `REPARSE_POINT`/硬链接/网络盘的一刀切拦截，以及重启恢复时的 NTFS `volume_serial_number + file_index` 强绑定；以常规同目录临时文件 + 原子重命名替代手写 `SetFileInformationByHandle` 与 4 阶段 2PC 协议；将 `last-workspace.sqlite` 与 `quick-entry-hotkey.sqlite` 合并为单个 `%LOCALAPPDATA%/.../settings.json`。
- **笔记与状态库演进（Schema v5）**：新随手记改为按本机日期追加到每日 Markdown 文件 `<YYYY-MM-DD>.md`（兼容历史一记一文件）；状态库精简至 Schema v5，分歧收敛为 Git Conflict 风格单表 `conflicts`，新增多轮对话表与轻量后台队列，保留已验证的增量扫描 + FTS5 trigram 混合检索引擎。
- **HTTP 栈统一**：在 `src-tauri/Cargo.toml` 显式声明 `Cargo.lock` 已锁定的 `reqwest`，替换手写 `unsafe` WinHTTP FFI，统一承载 LLM 整合、流式对话（SSE）与 OpenViking HTTP 接入（详见 [ADR-02](adr-02-model-and-memory.md)）。
