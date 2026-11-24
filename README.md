# Flash Marking Down

面向个人的 Windows Markdown 快记与记忆工作台，使用 Tauri 2、Vue 3、Rust 和 SQLite。当前开发版支持全局快捷键记录 Markdown、保存到本地工作区、离线搜索，以及配置模型后进行笔记整理、Wiki 变更审阅、待办和带来源的问答。

v1.0 计划加入按日文件、一屏三栏工作台、后台自动整理和 OpenViking 双源召回。开发阶段和完成标准见下方实施计划。

## 文档入口

- [开发规格 v1.0](SPECIFICATION.md)（产品定位、核心闭环与 FR-01~FR-07）
- [实施计划 v1.0](docs/spec/implementation-plan.md)（R1 / R2 / R3 重构路线与需求追踪）
- [数据与接口约定 v1.0](docs/spec/data-contracts.md)（每日 Markdown 格式、`settings.json`、Schema v5、`MemoryProvider` 与 OpenViking 路由映射）
- [验收规格 v1.0](docs/spec/acceptance.md)（AT-01~AT-12 务实用例与 E2E-Core）
- [UI 设计基准 DESIGN-01](docs/spec/ui-design.md) · [ADR-01 桌面选型](docs/spec/adr-01-options.md) · [ADR-02 模型与 OpenViking 接入](docs/spec/adr-02-model-and-memory.md)

## Windows 开发构建与测试

[Node.js 版本要求](package.json)和 [Rust 最低版本](src-tauri/Cargo.toml)见项目清单。Windows 构建还需要 MSVC C++ 工具链和 WebView2 Runtime。启动后选择工作区，再使用全局快捷键或托盘菜单打开快捷窗。

```powershell
npm ci --ignore-scripts
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --locked
npm run tauri build -- --debug --no-bundle -- --locked
.\src-tauri\target\debug\flash-marking-down.exe
```

手动测试请使用可丢弃的临时工作区和合成笔记。
