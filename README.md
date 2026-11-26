# Flash Marking Down

面向个人的 Windows Markdown 快记与记忆工作台，使用 Tauri 2、Vue 3、Rust 和 SQLite。当前开发版已具备一屏三栏主工作台（左栏记录 / Wiki / 待办三切、中栏 Markdown 预览 / 实时内联编辑 / 源码切换与内联 AI 整理、右栏可折叠多轮记忆对话与右侧冲突 / Wiki 辅助抽屉）、全局快捷键带实时语法渲染的按日快记（默认存放在工作区内 `notes/` 独立子目录）、离线全文检索，以及工作区本地相对路径图片安全预览。

启用模型后可在快记保存及原位修改后静默整理摘要、决定、待办与观点分歧，并在抽屉中用 Git 冲突式 Diff 决策和审阅 Wiki 变更建议。左栏 Wiki 页签支持按目录浏览、关键词筛选、内联新建与中栏阅读编辑映射。工作区可分别开启 OpenViking 每日原文同步和长期记忆提交。右侧对话可同时检索本地笔记与 OpenViking，点击引用可在中栏打开对应原文。OpenViking 不可用时，对话继续使用本地检索。

## 文档入口

- [开发规格 v1.0](SPECIFICATION.md)（产品定位、核心闭环与 FR-01~FR-07）
- [实施计划 v1.0](docs/spec/implementation-plan.md)（R1 / R2 / R3 重构路线与需求追踪）
- [数据与接口约定 v1.0](docs/spec/data-contracts.md)（每日 Markdown 格式、`settings.json`、Schema v6、`MemoryProvider` 与 OpenViking 路由映射）
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
