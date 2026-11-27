# Wikra v2.0 前后端冻结接口契约与验收标准

- **产品名称**：**Wikra**（独立通用的本地个人 LLM Wiki 与知识整理工作台，100% 兼容 Obsidian 本地 Markdown 文件）
- **版本与状态**：v2.0 Phase 1+2 核心闭环 **FROZEN（已冻结）** · 2026-11-26
- **目标工作区**：`D:/Development/flash-marking-down-v2-redesign`（分支 `redesign/v2-llm-wiki-agent`，严禁改动主仓库 `d:/Development/flash-marking-down`）
- **上游依据**：[产品定义书](../PRODUCT-DEFINITION-v2.md) · [需求设计 PRD](../REQUIREMENTS-PRD-v2.md) · [UI 设计规格书 Rev.3](ui-design/UI-DESIGN-SPEC-v2.md) · [高保真原型](ui-design/Wikra%20%E2%80%94%20Interactive%20Prototype.html)

---

## 1. 工程红线与零新增依赖约定

1. **零新增外部依赖**：严格复用当前 `package-lock.json`（Vue 3.5 / TS 5.9 / Vite 8 / CodeMirror 6 / `@tauri-apps/api` 2）与 `Cargo.lock`（Tauri 2.12 / `rusqlite` 0.43 FTS5 / `reqwest` 0.13 / `serde` / `sha2` / `tauri-plugin-dialog`）。
2. **反过度防御 5 条底线**：
   - 路径不越界：所有读写限定在工作区根目录内，扫描排除 `.wikra` 与历史 `.flash-marking-down`；
   - 写入不静默破坏：原子 temp 文件 + `sync_all` + rename；写已有 Wiki 文件前校验 `expectedHash`，磁盘被外部（如 Obsidian）改过返回 `VERSION_CONFLICT` 提示刷新；
   - 随手记不丢字不死锁：草稿防抖落盘 + `requestId` 幂等 + IME 组词期四级 `Esc` 保护 + 写盘失败始终提供复制全文与强制关闭逃生口；
   - 凭据不落盘：模型与 OpenViking API Key 仅存 Windows 凭据管理器，绝不写入配置文件、SQLite、日志或前端状态；
   - 渲染防 XSS：Markdown 预览禁止执行脚本，不自动加载未授权远端图片（仅通过 `read_workspace_image` 加载工作区本地相对路径图片）。
3. **命名与兼容**：
   - 应用标题、打包产品名统一为 **`Wikra`**（`tauri.conf.json`：`productName: "Wikra"`, `title: "Wikra"`）；
   - 新建标准库使用隐藏状态目录 **`.wikra`**；打开历史已存在 `.flash-marking-down` 的旧工作区时自动兼容识别，文档扫描同时排除 `.wikra` 与 `.flash-marking-down`；
   - 受控 Frontmatter 与手动保护注释统一支持 `wikra-core` / `wikra-managed` / `wikra_pending_conflicts` / `<!-- wikra-protect:start -->` ~ `<!-- wikra-protect:end -->`（同时向下兼容读取旧 `fmd-*` 前缀）。

---

## 2. 应用配置扩展（`settings.rs` · `AppSettings`）

保留 `version: 1` 并通过 `#[serde(default)]` 向后兼容新增字段（camelCase JSON）：

```ts
export interface AppSettings {
  version: number
  lastWorkspaceRoot: string | null
  lastCaptureDir: string
  hotkey: string
  autoOrganize: boolean
  autoSyncMemory: boolean
  theme: 'slate' | 'paper'           // 默认 'slate'（深色）；'paper'（浅色）
  docWidth: 'readable' | 'full'      // 默认 'readable'（920px）；'full'（100%）
  allowNoteFulltextToModel: boolean  // 默认 true
  quietOrganize: boolean             // 默认 true
  bufferSeconds: number              // 默认 60（允许 0..300）
}
```

- `save_app_settings` 扩展接收上述全部可选参数并持久化至 `settings.json`。

---

## 3. 冻结 Tauri IPC 命令与 TypeScript / Rust DTO 契约

所有命令均遵循现有统一错误信封 `AppError { code, stage, operationId, retryable, safeMessage, technicalDetail }`，JSON 字段一律 `camelCase`。既有命令保持向后兼容，新增与扩展契约如下：

### 3.1 知识库初始化与统一文档查看器（Wiki + 原始文件同树并存）

1. **`init_standard_vault(root: String) -> OpenedWorkspaceDto`**
   - 在已选绝对目录 `root` 下安全创建标准 Wikra 目录（已存在则不覆盖）：`wiki/decisions`、`wiki/concepts`、`wiki/syntheses`、`wiki/entities`、`raw/notes`、`raw/clips`，如无 `SCHEMA.md` 则写入简洁默认 `SCHEMA.md`，随后打开该工作区（`capture_dir = "raw/notes"`）。
2. **`list_library_tree(session: String) -> LibraryTreeDto`**
   ```ts
   export interface LibraryDocSummary {
     path: string
     title: string
     folder: string
     docType: 'decision' | 'concept' | 'synthesis' | 'entity' | 'note' | string
     typeLabel: string
     tags: string[]
     updatedAt: string | null
     citationCount: number
     pendingConflictCount: number
     wikraCore: boolean
   }
   export interface RawFileSummary {
     id: string
     path: string
     title: string
     kind: 'note' | 'clip'
     kindLabel: '随手笔记' | '收藏原文'
     createdAt: string | null
     sourceUrl: string | null
     linkedPagePaths: string[]
   }
   export interface LibraryTree {
     wikiPages: LibraryDocSummary[]
     rawFiles: RawFileSummary[]
   }
   ```
   - 归类规则：位于 `raw/` 或工作区 `captureDir` 或登记在 `notes` 捕获表中的 Markdown 文件归入 `rawFiles`；其余 Markdown 归入 `wikiPages`。
3. **`read_document_detail(session: String, relativePath: String) -> DocumentDetailDto`**
   ```ts
   export interface DocCitation {
     id: string
     refLabel: string     // 如 "s1"
     rawPath: string      // 指向工作区内原文相对路径（若存在）
     label: string
     quote: string
   }
   export interface DocBacklink {
     pagePath: string
     title: string
     excerpt: string
   }
   export interface DocTaskItem {
     id: string
     lineNumber: number
     expectedLine: string
     text: string
     done: boolean
   }
   export interface DocumentDetail {
     kind: 'wiki' | 'raw'
     path: string
     title: string
     body: string
     contentHash: string
     updatedAt: string | null
     docType: string
     typeLabel: string
     aliases: string[]
     tags: string[]
     wikraCore: boolean
     sourceUrl: string | null
     pendingConflictIds: string[]
     outline: string[]
     tasks: DocTaskItem[]
     citations: DocCitation[]
     backlinks: DocBacklink[]
     linkedWikiPaths: string[] // 仅 raw 文件非空：引用/整理自该原文的 wiki 页
     hasProtectedSection: boolean
   }
   ```
4. **`save_markdown_document(session: String, relativePath: String, expectedHash: String, body: String) -> DocumentDetailDto`**
   - 校验路径在工作区内且非 `.wikra`；比对当前磁盘文件 SHA-256 与 `expectedHash`，不匹配返回 `code = "VERSION_CONFLICT"`；匹配则原子写入并增量更新 FTS5 索引，返回最新 `DocumentDetailDto`。
5. **`create_markdown_document(session: String, relativePath: String, initialBody: String) -> DocumentDetailDto`**
   - 在工作区安全路径新建 `.md` 文件（已存在则返回 `VERSION_CONFLICT`），更新索引并返回详情。
6. **`append_human_understanding(session: String, relativePath: String, insightText: String) -> DocumentDetailDto`**
   - 将用户的领悟追加到该页受保护区间 `<!-- wikra-protect:start -->` ~ `<!-- wikra-protect:end -->`（若尚无该章节则自动在文末脚注前追加），原子写盘并返回最新文档。

### 3.2 收件箱（原文阅读器、网页抓取降级与分段提炼）

7. **`list_inbox_items(session: String) -> InboxItemSummaryDto[]`**
8. **`get_inbox_item(session: String, id: String) -> InboxItemDetailDto`**
   ```ts
   export interface InboxParagraph {
     id: string
     heading: string
     text: string
     highlighted: boolean
     compiled: boolean
   }
   export interface InboxItemSummary {
     id: string
     title: string
     domain: string
     sourceType: string
     url: string | null
     rawPath: string
     savedAt: string
     readTime: string
     fetchStatus: 'full' | 'stub'
     state: 'unread' | 'reading' | 'compiled' | 'archived'
     wordCount: number
     highlightCount: number
   }
   export interface InboxItemDetail extends InboxItemSummary {
     summaryPoints: string[]
     relatedPagePaths: string[]
     paragraphs: InboxParagraph[]
   }
   ```
9. **`ingest_inbox_input(session: String, input: String, titleHint?: String) -> InboxItemDetailDto`**
   - 输入为 `http://` 或 `https://` URL 时：通过 `reqwest` 抓取正文（绝不读取浏览器 Cookie、绝不绕过登录墙）；成功解析正文则写不可变原件至 `raw/clips/YYYY/MM/<slug>.md`（`fetchStatus = "full"`），遇到 401/403/登录墙/抓取失败则保存元信息收据（`fetchStatus = "stub"`）。
   - 输入为普通文本/Markdown 时：直接写不可变原件至 `raw/clips/YYYY/MM/<slug>.md`（`fetchStatus = "full"`），分段并生成基础摘要点。
10. **`complete_inbox_stub(session: String, id: String, pastedBody: String) -> InboxItemDetailDto`**
    - 将用户手动粘贴的正文补全写入原件文件并转为 `fetchStatus = "full"`、切分段落。
11. **`toggle_inbox_paragraph(session: String, id: String, paragraphId: String) -> InboxItemDetailDto`**
    - 切换段落的 `highlighted` 状态并持久化到 SQLite（已 `compiled` 段落不可取消）。
12. **`compile_inbox_item(session: String, id: String, mode: 'excerpt' | 'full' | 'archive') -> InboxItemDetailDto`**
    - `archive`：仅标为已存档，不动原件；
    - `excerpt`：仅提取 `highlighted && !compiled` 段落送入整理管线；无未提炼重点时返回 `INVALID_INPUT`；
    - `full`：全篇送入整理管线；
    - 整理完成后标记对应段落 `compiled = true`，生成可撤销的 `Changeset` 记录并同步更新关联 Wiki 页（若模型未启用则返回清晰可操作错误，绝不静默丢数据）。
13. **`open_external_url(url: String) -> ()`**
    - 严格校验仅允许 `http://` 或 `https://` 协议，调用 Windows 系统默认浏览器打开原网页。

### 3.3 整理中心、60 秒缓冲队列、可撤销 Changeset 与主动回忆

14. **`list_organize_overview(session: String) -> OrganizeOverviewDto`**
    ```ts
    export interface ChangesetOp {
      label: string
      targetPath: string
      targetTitle: string
      outcome: 'applied' | 'pending_review' | 'reverted'
    }
    export interface ChangesetItem {
      id: string
      time: string
      trigger: string
      modelAlias: string
      status: 'applied' | 'partial' | 'reverted'
      summary: string
      pagePaths: string[]
      ops: ChangesetOp[]
    }
    export interface BufferQueueItem {
      id: string
      noteId: string
      path: string
      title: string
      preview: string
      queuedAt: string
    }
    export interface OrganizeSuggestion {
      id: string
      kind: 'review' | 'orphan'
      kindLabel: string
      title: string
      detail: string
      actionLabel: string
      pagePath: string
    }
    export interface OrganizeOverview {
      conflicts: ConflictItem[]
      changesets: ChangesetItem[]
      suggestions: OrganizeSuggestion[]
      bufferQueue: BufferQueueItem[]
      bufferSecondsLeft: number
    }
    ```
15. **`commit_capture_v2(session: String, requestId: String, revision: number, body: String, mode: 'only' | 'queue' | 'now') -> CaptureCommitResultDto`**
    - 复用幂等 `requestId` 保存至本地原始随手记文件（写入 `raw/notes/...` 或工作区 `captureDir`）；
    - `only`：仅存原文不触发整理；
    - `queue`：进入不顺延 60 秒合批缓冲队列（倒计时归零后台自动触发合并整理，并向 `main` 发射 `wikra-buffer-updated` / `wikra-organize-finished` 事件）；
    - `now`：立即入队并唤醒后台整理器。
    - 返回 `{ path, noteId, queued, bufferSecondsLeft }`。
16. **`cancel_buffer_item(session: String, itemId: String) -> OrganizeOverviewDto`** / **`flush_buffer_now(session: String) -> OrganizeOverviewDto`**
    - 支持在倒计时结束前将单条快记移出本次整理（保留本地原文）或立刻触发整理。
17. **`revert_changeset(session: String, changesetId: String) -> ChangesetItem`**
    - 根据保存的 `before_snapshot` 安全回滚受影响 Wiki 文件（保留受保护区间或冲突时提示），标记 `status = "reverted"`。
18. **`dismiss_suggestion(session: String, suggestionId: String) -> ()`**
19. **`teachback_turn(session: String, pagePath: String, step: number, userAnswer?: String) -> TeachbackTurnDto`**
    - `{ step, totalSteps: 3, prompt, feedback, suggestedInsight: string | null }`。有模型配置时由模型结合该页正文与引文生成针对性边界追问；未配置模型时优雅降级为基于该页章节骨架与引文的本地结构化追问，确保离线可体验。
20. **`save_answer_to_wiki(session: String, conversationId: String, messageId: String, targetPath?: String) -> DocumentDetailDto`**
    - 将 AI 问答回复连同出处引用沉淀写入指定或新建的 `wiki/syntheses/` 页面。

### 3.4 交互知识图谱与三源全局检索

21. **`get_knowledge_graph(session: String, includeMetaPages: boolean) -> KnowledgeGraphDto`**
    ```ts
    export interface GraphNode {
      id: string      // relative path
      title: string
      docType: string
      degree: number
      isHub: boolean
      isOrphan: boolean
    }
    export interface GraphEdge {
      id: string
      from: string
      to: string
      kind: 'link' | 'citation'
      label: string
      snippet: string
    }
    export interface GraphGap {
      id: string
      title: string
      reason: string
      question: string
      fromPath: string
      toPath: string
    }
    export interface KnowledgeGraph {
      nodes: GraphNode[]
      edges: GraphEdge[]
      gaps: GraphGap[]
    }
    ```
22. **`dismiss_graph_gap(session: String, gapId: String) -> ()`**
23. **`global_search(session: String, query: String, limit?: number) -> GlobalSearchHitDto[]`**
    ```ts
    export interface GlobalSearchHit {
      kind: 'wiki' | 'raw' | 'citation'
      kindLabel: '知识页' | '随手笔记' | '收藏原文' | '原文引文'
      title: string
      subtitle: string
      targetPath: string
      citationId: string | null
    }
    ```
    - 严格实现 **三源覆盖**（`wiki/` 标题与正文、`raw/` 完整原文、已提取的原文引文短引），跨任意视图可随时调用。

---

## 4. 前后端并行分工边界（严禁越界修改）

| 角色 | 委派通道与模型 | 允许修改的文件范围 | 核心交付物 |
|---|---|---|---|
| **后端开发** | Codex CLI (`gpt-6.2-sol`, `model_reasoning_effort="max"`) | `src-tauri/` 下 Rust 源码与 `src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`（不加新 crate） | 完整实现 §2–§3 全部后端命令、SQLite schema v7 迁移、HTML 抓取与登录墙降级、Changeset 回滚、60s 缓冲队列、图谱解析、三源搜索、Teachback 与全部 Rust 单元测试 |
| **前端开发** | Antigravity (`gemini-4-neon-high`) | `index.html`、`src/` 下全部 Vue / TS / CSS 文件（不含 `src-tauri/`） | 完整实现 Wikra Rev.3 桌面 UI（A 深色 + C 浅色、宽屏 1920/1440/1100 适配、统一知识库 + 原文分屏、Readwise 收件箱阅读器、精简两栏「整理」、多页冲突对比、交互图谱、双页签右栏、随手记浮窗升级、设置与接库向导）并与 `src/api/client.ts` 全量对接 |

---

## 5. 验收门禁标准（进入交付前必须 100% 达标）

1. **后端质量门禁**：
   - `cargo fmt --manifest-path src-tauri/Cargo.toml --check` 通过；
   - `cargo test --manifest-path src-tauri/Cargo.toml --locked` 全量通过（涵盖既有测试 + 新增 Wikra v2.0 单元测试：标准库初始化、Wiki/Raw 同树识别、乐观锁冲突保护、Inbox 抓取与登录墙降级、分段高亮与提炼、Changeset 撤销、图谱解析与断层检测、三源搜索、60s 缓冲移出/刷新、`wikra-protect` 区间保护与追加、URL 安全校验）。
2. **前端构建与类型门禁**：
   - `npm run build`（`vue-tsc --noEmit && vite build`）零错误、零未定义类型通过；
   - 生产构建中不包含原型左下角调试「预览」按钮；A/C 双主题与 `适读宽 / 铺满宽` 持久化至 `AppSettings`；
   - 后台定时刷新绝不冲毁 `INPUT` / `TEXTAREA` / CodeMirror (`contenteditable`) 活跃焦点与中文 IME 组词态；主动回忆遮蔽幕布按 `pagePath` 隔离、切页自动复位。
3. **Windows 桌面构建门禁**：
   - `npm run tauri build -- --debug --no-bundle -- --locked` 成功产出可执行程序 `src-tauri/target/debug/wikra.exe`（或对应二进制名），无链接/资源错误。
4. **总控代码审查与仓库卫生**：
   - 主控亲自逐项审查前后端 diff，确认严格遵守反过度防御红线、无凭据落盘、无 XSS 漏洞、主仓库 `d:/Development/flash-marking-down` 零改动，不擅自执行 `git commit`。
