# Wikra

面向个人的 Windows 本地优先 **LLM Wiki 知识工作台与 Agent**（原 Flash Marking Down 演进至 v2.0），基于 Tauri 2、Vue 3、Rust 与 SQLite 构建，100% 兼容 Obsidian 本地 Markdown 文件格式。

核心闭环：任意窗口通过全局快捷键唤出浮窗零摩擦秒记或投递文章/链接到收件箱 → 不可变落盘 `raw/` 原件并按需勾选重点段落 → 后台整理生成带一跳逐字短引的可回滚 `Changeset` 与冲突对比 → 在一体化三栏工作台中浏览/编辑知识库、通过交互图谱发现结构关联、用「让我讲一遍」主动内化，并结合本地检索与 OpenViking 进行多轮对话召回。

## 当前 v2.0 核心能力

- **100% Obsidian 兼容与自定义目录**：新建标准 Vault（`raw/`、`wiki/`、`SCHEMA.md`、`.wikra/`）或原位接入任意已有 Obsidian 库（绝不搬移或覆写存量文件），支持按库自定义 `raw` 原始文件目录与随手记保存目录，向下兼容历史 `.flash-marking-down` 工作区。
- **统一知识库与原文查看器**：高密度可折叠多级目录树同时呈现知识页与原始文件；支持阅读/源码切换、Markdown 安全预览、本地相对路径图片、反链与一跳原文证据对照、受保护人工段落（`<!-- wikra-protect:start/end -->`）以及明暗双主题（Slate / Paper）。
- **全局秒记浮窗与 60s 合批缓冲**：隐藏窗口复用（热唤出 p95 < 100 ms）、中文 IME 组词四级 `Esc` 保护、写盘失败防死锁逃生，支持「仅存原文 / 入 60s 合批缓冲 / 立即整理」。
- **Readwise 风格收件箱**：粘贴 URL 或文本写不可变原件至 `raw/clips/`，遇登录墙透明降级为链接收据并支持补全正文；支持段落级勾选「仅编选段 / 全篇整理 / 仅存档」。
- **整理中心、知识图谱与主动回忆**：可一键撤销的 `Changeset` 与写前快照、Git Merge Conflict 风格冲突裁决、交互式知识图谱、3 步遮答式「让我讲一遍」回顾（确认后写入 `## 我的理解`）以及本地 FTS5 + OpenViking 双源对话与「沉淀入 Wiki」。

## 文档入口

- [Wikra v2.0 产品定义书](docs/v2-redesign/PRODUCT-DEFINITION-v2.md)（第一性原理定位、五大支柱、Vault 契约与待验证假设）
- [Wikra v2.0 需求设计 PRD](docs/v2-redesign/REQUIREMENTS-PRD-v2.md)（核心功能模块、分层自治、双记忆协同、MCP 与演进路线）
- [Wikra v2.0 冻结接口契约与验收标准](docs/v2-redesign/INTERFACE-CONTRACTS-AND-ACCEPTANCE-v2.md)
- [Wikra UI 设计规格书](docs/v2-redesign/ui-design/UI-DESIGN-SPEC-v2.md) · [多模型方案比对合成记录](docs/v2-redesign/MULTI-MODEL-SYNTHESIS.md)
- 历史 v1.0 规格归档：[SPECIFICATION.md](SPECIFICATION.md) · [ADR-01](docs/spec/adr-01-options.md) · [ADR-02](docs/spec/adr-02-model-and-memory.md)

## Windows 开发构建与测试

[Node.js 版本要求](package.json)和 [Rust 最低版本](src-tauri/Cargo.toml)见项目清单。Windows 构建还需要 MSVC C++ 工具链和 WebView2 Runtime。

```powershell
npm ci --ignore-scripts
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo test --manifest-path src-tauri/Cargo.toml --locked
npm run tauri build -- --debug --no-bundle -- --locked
.\src-tauri\target\debug\wikra.exe
```

手动测试请使用可丢弃的临时工作区和合成笔记。
