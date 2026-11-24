# 数据与接口约定 v1.0

更新于 2026-11-24，与 [开发规格 v1.0](../../SPECIFICATION.md) 配套。本文件定义精简务实的本地存储格式、SQLite Schema v5、连接器抽象、OpenViking HTTP 映射与 Tauri IPC 命令分组。

---

## 1. 本地文件布局与每日笔记格式

### 1.1 目录结构
- **应用全局配置**（`%LOCALAPPDATA%/<app_id>/settings.json`）：
  取代原有的 `last-workspace.sqlite` 与 `quick-entry-hotkey.sqlite`，单一 JSON 文件原子写回：
  ```json
  {
    "version": 1,
    "lastWorkspaceRoot": "D:/Notes/my-workspace",
    "lastCaptureDir": "inbox",
    "hotkey": "win-alt-n",
    "autoOrganize": true,
    "autoSyncMemory": true
  }
  ```
- **工作区目录**（用户选定的任意本地目录，允许位于 OneDrive/云盘同步目录、Junction 或移动盘，支持直接拷贝迁移）：
  - `<capture_dir>/<YYYY-MM-DD>.md`：按本机日期归集的每日随手记文件；
  - `<workspace>/**/*.md`：工作区内所有其他普通 Markdown 文件与历史一记一文件（均参与离线检索与对话召回）；
  - `<workspace>/.flash-marking-down/state.sqlite`：工作区应用状态与派生搜索索引（自动排除在扫描、整理与外部同步之外）。

### 1.2 每日随手记块格式
为兼顾 Obsidian 等外部编辑器的自然阅读体验与单条笔记精准溯源，每日文件采用如下 Markdown 结构：

```markdown
# 2026-11-24

## 15:03:12 <!-- fmd-note:a1b2c3d4 -->

今天讨论倾向先沿用现有恢复方案，移动端适用性待验证。
- [ ] 周五前找后端核对接口

## 16:40:05 <!-- fmd-note:e5f60718 -->

关于 OpenViking 同步：先写 resources 保留原文，再提交 session 抽长期记忆。
```

- **块定位规则**：应用优先通过 `<!-- fmd-note:<8~16位短ID> -->` 识别单条随手记块；若用户在外部编辑器手动删除了注释或手动新增了普通小节，仍作为普通 Markdown 正常索引和阅读，不报错。
- **历史兼容**：工作区内旧版一记一文件（带 `record_id` / `source_id` frontmatter 或无 frontmatter 的 `.md`）全部兼容读取、展示与搜索，不强迫迁移原文件。

### 1.3 务实写入约定
- **写新记录（每日文件追加）**：
  1. 读取目标 `<YYYY-MM-DD>.md` 当前内容（若不存在则初始化 `# YYYY-MM-DD\n`）；
  2. 在末尾规范化换行后拼接新块；
  3. 写入同目录临时文件 `.<filename>.fmd.tmp` → `sync_all()` → 原子重命名覆盖目标文件；
  4. 更新 `state.sqlite` 的 `notes` 记录、增量搜索索引，并投递后台整合任务。
- **改写已有文件（Wiki 采纳 / 待办勾选）**：
  - 写入前校验当前磁盘内容哈希（Wiki 采纳）或目标任务行内容（待办勾选）；若与预期一致则直接临时文件原子替换，若已被外部编辑器修改则中止写入并提示刷新/重新比较，绝不静默覆盖人工编辑。
  - **不再长期持有父目录防删句柄**，不再绑定 NTFS Volume Serial / File Index。

---

## 2. 工作区 SQLite Schema v5（精简模型）

打开旧版 v1–v4 工作区时，在单事务内平滑升级至 v5（保留已有草稿、搜索文档、Wiki 映射与历史记录；不再依赖 `capture_intents` 2PC 表、`m2_preparations` 与重型分歧三表）。

| 表名 | 核心字段 | 用途 |
|---|---|---|
| `workspace_settings` | `key`, `value` | `workspace_id`、`capture_dir`、模型配置（`endpoint`, `model`, `enabled`）、OpenViking 配置（`base_url`, `namespace`, `enabled`, `sync_resources`, `commit_memories`）、凭据引用名 |
| `drafts` | `id`, `body`, `edit_revision`, `updated_at` | 快捷窗草稿防抖落盘；关闭/退出失败时允许用户复制并强制清除 |
| `notes` | `id`, `request_id` (UNIQUE), `relative_path`, `block_id`, `captured_at`, `title`, `body_preview`, `content_hash`, `organize_status`, `organize_json`, `ov_resource_status`, `ov_resource_uri`, `ov_memory_status`, `ov_task_id`, `updated_at` | 随手记条目索引与轻量状态投影（整合摘要/决定/待办/状态、OV 双路同步状态）；`request_id` 防双击重复 |
| `search_documents` + `search_documents_fts` | `relative_path`, `captured_at`, `revision`, `body`, `mtime_ns`, `file_size` + FTS5 trigram 虚拟表与触发器 | 沿用已验证的高性能增量扫描 + FTS5/字面混合检索（5,000 篇增量重建 < 100 ms，查询 p95 < 30 ms） |
| `conflicts` | `id`, `topic`, `current_label`, `current_path`, `current_text`, `incoming_label`, `incoming_path`, `incoming_text`, `status` (`open` \| `keep_current` \| `accept_incoming` \| `keep_both`), `resolution_note`, `created_at`, `resolved_at` | **Git Conflict 风格分歧单表**；每条独立容错，无全局熔断 |
| `wiki_targets` | `path`, `kind` (`wiki` \| `tasks`) | 映射的 Wiki 页面与唯一 Markdown 任务清单（支持添加与**解除映射**） |
| `wiki_suggestions` | `id`, `note_id`, `target_path`, `base_hash`, `before_text`, `after_text`, `reason`, `status` (`pending` \| `applied` \| `rejected` \| `reverted`), `created_at` | 仅存真正的 Wiki 文档变更建议与撤销依据，**不混入任务勾选流水** |
| `conversations` / `chat_messages` | `conversations(id, title, created_at, updated_at)`；`chat_messages(id, conversation_id, role, content, citations_json, conflict_ids_json, created_at)` | 右侧常驻多轮对话历史持久化与引文/冲突关联 |
| `background_jobs` | `id`, `kind` (`organize` \| `ov_write_resource` \| `ov_commit_memory`), `note_id`, `payload_json`, `status` (`queued` \| `running` \| `done` \| `failed`), `attempts`, `last_error`, `updated_at` | 轻量单队列异步作业表，重启自动续跑或供界面一键重试 |

---

## 3. 记忆连接器契约与 OpenViking v0.5.2 映射

### 3.1 Rust 接口抽象（`src-tauri/src/memory/mod.rs`）

```rust
pub struct MemoryHealth {
    pub healthy: bool,
    pub version: Option<String>,
    pub identity: Option<String>, // 如 account_id / user_id
}

pub struct MemoryHit {
    pub uri: String,
    pub kind: String,             // "memory" | "resource"
    pub title: String,
    pub snippet: String,
    pub score: Option<f64>,
    pub updated_at: Option<String>,
}

pub trait MemoryProvider: Send + Sync {
    fn provider_id(&self) -> &'static str;
    fn health_check(&self) -> Result<MemoryHealth, AppError>;
    fn upsert_daily_note(&self, relative_uri: &str, markdown: &str, tags: &[String]) -> Result<String, AppError>;
    fn commit_note_memory(&self, title: &str, markdown: &str, tags: &[String]) -> Result<Option<String>, AppError>;
    fn search(&self, query: &str, limit: usize, since: Option<&str>, until: Option<&str>) -> Result<Vec<MemoryHit>, AppError>;
    fn read_content(&self, uri: &str) -> Result<String, AppError>;
}
```

### 3.2 `OpenVikingProvider` 真实路由映射（已核对本机 v0.5.2 源码）
所有请求统一复用 `reqwest` 客户端；认证支持配置 `X-API-Key`（默认）或 `Authorization: Bearer <key>`，凭据读取自 Windows 凭据管理器。

| Trait 方法 | HTTP 方法与路径 | 关键请求/响应约定 |
|---|---|---|
| `health_check` | `GET /health` | 携带认证头，返回 `status == "ok"`、`version`、`account_id` / `user_id`；供设置页「测试连接」使用 |
| `upsert_daily_note` | `POST /api/v1/content/write` | Body：`{ "uri": "viking://resources/flash-marking-down/<workspace>/daily/YYYY-MM-DD.md", "content": "...", "mode": "replace", "wait": false, "tags": [...] }`；后台异步索引 |
| `commit_note_memory` | `POST /api/v1/sessions` → `POST /api/v1/sessions/{id}/messages` → `POST /api/v1/sessions/{id}/commit` | 创建短会话写入用户记录与整理摘要，触发 `commit`（`wait: false`）由 OV 抽取长期记忆，返回可选 `task_id` |
| `search` | `POST /api/v1/search/search`（或 `/api/v1/search/find`） | 传 `query`、`limit`、可选 `since`/`until` 与 `target_uri`，返回命中的 `memories` 与 `resources` 列表 |
| `read_content` | `GET /api/v1/content/read?uri=...`（或 `/overview`） | 按需展开命中记忆或资源的详细内容供中栏预览或对话上下文补全 |

---

## 4. Tauri IPC 命令分组与错误契约

沿用强类型 camelCase DTO 与统一 `AppError`（`{ code, stage, operationId?, retryable, safeMessage, technicalDetail? }`），默认向界面呈现简洁中文 `safeMessage`，技术细节折叠于 `<details>`。

| 分组 | 核心命令 | 说明 |
|---|---|---|
| **工作区与设置** | `resume_workspace`, `open_workspace`, `get_app_settings`, `save_app_settings`, `get_hotkey`, `set_hotkey`, `pick_workspace_folder`, `pick_workspace_markdown` | 统一读写 `settings.json` 与工作区配置，快捷窗打开时切换工作区先自动落盘隐藏再切换 |
| **模型与 OV 配置** | `get_integrations_config`, `save_model_config`（含可选一次性 API Key）, `test_model_connection`, `save_memory_config`（含可选一次性 API Key）, `test_memory_connection` | 模型与 OV 均支持“一屏一次保存 + 一键测试连接”；密钥仅走 Windows `CredWriteW`（UTF-16 字节上限安全校验） |
| **快记与草稿** | `show_capture_window`, `load_capture_workspace`, `persist_draft`, `commit_capture`, `close_capture_window` | `commit_capture` 原子追加当日 `.md` 文件、更新索引并入队后台整合；支持强关逃生 |
| **记录与检索** | `list_notes`, `search_records`, `rebuild_search_index`, `get_note_detail`, `open_workspace_file` | 聚合原文、内联整理摘要、关联待办、OV 同步态与关联冲突 ID，一次返回消除 N+1 |
| **后台整合与待办** | `trigger_note_organize`, `retry_memory_sync`, `list_tasks`, `toggle_task` | `toggle_task` 直接轻快改写 Markdown 任务行（兼容手工 `- [ ]`）；后台进度通过 Tauri 事件 `note-updated` / `drawer-badge-updated` 推送前端 |
| **冲突与 Wiki 抽屉** | `list_conflicts`, `create_conflict`, `resolve_conflict`（`keep_current \| accept_incoming \| keep_both`）, `wiki_targets`, `map_wiki_target`, `unmap_wiki_target`, `list_wiki_suggestions`, `apply_wiki_suggestion`, `reject_wiki_suggestion`, `undo_wiki_suggestion` | Git 冲突三选一处理；支持解除 Wiki 映射；Wiki 建议列表不再混入任务勾选记录 |
| **多轮记忆对话** | `list_conversations`, `get_conversation`, `delete_conversation`, `send_chat_message`（流式事件 `chat-chunk` / `chat-done`）, `read_memory_item` | 本地 FTS5 + OpenViking 双源混合召回，流式输出带引用角标与关联冲突卡片的回答 |
