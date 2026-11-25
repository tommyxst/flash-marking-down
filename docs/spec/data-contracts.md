# 数据与接口约定 v1.0

更新于 2026-11-25，与 [开发规格 v1.0](../../SPECIFICATION.md) 配套。本文件定义精简务实的本地存储格式、SQLite Schema v6、连接器抽象、OpenViking HTTP 映射与 Tauri IPC 命令分组。

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
- R1 的最近记录返回物理 `path` 和可选 `blockId`。读取原文、整理及引用可使用 `<path>#fmd-note:<id>` 定位一条正文。全文索引仍按文件维护，被手动删除标记的内容仍可全文搜索。

### 1.3 务实写入约定
- **写新记录（每日文件追加）**：
  1. 读取目标 `<YYYY-MM-DD>.md` 当前内容（若不存在则初始化 `# YYYY-MM-DD\n`）；
  2. 在末尾规范化换行后拼接新块；
  3. 写入同目录临时文件 → `sync_all()` → 比对当前文件内容 → 原子重命名；
  4. 更新 `state.sqlite` 的 `notes` 记录与增量搜索索引。后台整理和同步投递分别在 R2 和 R3 接入。
- **改写已有文件（Wiki 采纳 / 待办勾选）**：
  - Wiki 采纳校验预览时的完整内容。待办勾选先比对清单中的目标行，再原子写回，保留其他行的外部修改。如果同时出现新的文件写入，提示刷新或重新比较。
  - 待办定位符含 `#fmd-note:<id>` 时，`line:N` 是块正文内从 1 开始的行号。整文件路径使用文件行号。写盘成功后，同文件已同步的原文状态回到 `pending`，自动补推遵循 `auto_sync_memory`。
  - 应用闲置时关闭 SQLite 文件连接，工作区通过路径打开。内网 UNC 共享使用 SQLite 回滚日志，本机工作区使用 WAL。

---

## 2. 工作区 SQLite Schema v6（精简模型）

打开旧版 v1–v5 工作区时，在单事务内平滑升级至 v6（保留已有草稿、搜索文档、Wiki 映射、历史记录与已有建议/分歧；移除遗留的 `capture_intents` 2PC 表、`m2_preparations`、重型分歧三表，以及 `sources`、`source_revisions`、`jobs`、`m2_analyses`、`m2_answers` 过渡表）。

旧捕获的保存记录迁入 `notes`，`block_id` 为空。已有一记一文件保持原样。仅保存在旧写入意图中的未完成正文并入现有草稿。旧分歧的观点、判断和历史存入 `conflicts.legacy_json`。`user_version = 6` 将 `wiki_suggestions` 正式收敛为下表 9 列精简结构（`prepared` 归并或迁移后统一为 `pending | applied | rejected | reverted`）。

| 表名 | 核心字段 | 用途 |
|---|---|---|
| `workspace_settings` | `key`, `value` | `workspace_id`、`capture_dir`、模型配置（`endpoint`, `model`, `enabled`）、OpenViking 配置（`base_url`, `namespace`, `enabled`, `sync_resources`, `commit_memories`）、凭据引用名 |
| `drafts` | `id`, `body`, `edit_revision`, `updated_at` | 快捷窗草稿防抖落盘；关闭/退出失败时允许用户复制并强制清除 |
| `notes` | `id`, `request_id` (UNIQUE), `relative_path`, `block_id`, `captured_at`, `title`, `body_preview`, `content_hash`, `organize_status`, `organize_json`, `ov_resource_status`, `ov_resource_uri`, `ov_memory_status`, `ov_task_id`, `updated_at` | 随手记条目索引与轻量状态投影（整合摘要/决定/待办/状态、OV 双路同步状态）；`request_id` 防双击重复 |
| `search_documents` + `search_documents_fts` | `relative_path`, `captured_at`, `revision`, `body`, `mtime_ns`, `file_size` + FTS5 trigram 虚拟表与触发器 | 沿用已验证的高性能增量扫描 + FTS5/字面混合检索（AT-12 基线：5,000 篇单文件增量重建 ~108 ms，查询 p95 ~17.4 ms） |
| `conflicts` | `id`, `topic`, `current_label`, `current_path`, `current_text`, `incoming_label`, `incoming_path`, `incoming_text`, `status` (`open` \| `keep_current` \| `accept_incoming` \| `keep_both`), `resolution_note`, `created_at`, `resolved_at` | **Git Conflict 风格分歧单表**；每条独立容错，无全局熔断 |
| `wiki_targets` | `path`, `kind` (`wiki` \| `tasks`) | 映射的 Wiki 页面与唯一 Markdown 任务清单（支持添加与**解除映射**） |
| `wiki_suggestions` | `id`, `note_id`, `target_path`, `base_hash`, `before_text`, `after_text`, `reason`, `status` (`pending` \| `applied` \| `rejected` \| `reverted`), `created_at` | 仅存真正的 Wiki 文档变更建议与撤销依据，**不混入任务勾选流水** |
| `conversations` / `chat_messages` | `conversations(id, title, created_at, updated_at)`；`chat_messages(id, conversation_id, role, content, citations_json, conflict_ids_json, created_at)` | 持久化多轮对话历史、引文、冲突关联和降级提示。`citations_json` 可读取现有引用数组，也支持含 `citations` 与 `notice` 的对象 |
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
    pub kind: String,             // "ov_memory" | "ov_resource"
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
| `health_check` | `GET /health` | 携带认证头，返回 `status == "ok"`、`version`、`account_id` / `user_id`、`role`；供设置页「测试连接」使用（非 `dev` 模式下拒绝不可读写业务数据的 `root` 管理密钥） |
| `upsert_daily_note` | `POST /api/v1/content/write` | 请求含 `uri`、`content`、`mode: "replace"`、`wait: false`。合规标签为无空白的小写 `key=value`，无合规标签时省略。每日文件用 `daily/YYYY-MM-DD.md`，手动同步的普通 Markdown 用 `notes/<relative_path>` |
| `commit_note_memory` | `POST /api/v1/sessions` → `POST /api/v1/sessions/{id}/messages` → `POST /api/v1/sessions/{id}/commit` | 创建与提交传 `{}`，消息传 `{"role":"user","content":"…"}`。提交返回可选 `task_id`，长期记忆由 OV 后台抽取 |
| `search` | `POST /api/v1/search/search` | 传 `query`、`limit`、`mode: "list"`、`context_type: ["memory","resource"]`、`read_content: true` 与可选 `since`/`until`。在当前凭据可访问的范围内检索，标题和片段由命中的正文或摘要生成 |
| `read_content` | `GET /api/v1/content/read?uri=...`（或 `/overview`） | 按需展开命中记忆或资源的详细内容供中栏预览或对话上下文补全 |

业务接口使用 `status: "ok"` 和 `result` 响应包裹，`/health` 使用顶层健康状态。OpenViking 的 `CONFLICT` 和 HTTP 409 标为可重试。服务接受请求后记录 `synced`。原文上传期间文件如有变化，保留待同步状态。异步索引和抽取由服务处理。

---

## 4. Tauri IPC 命令分组与错误契约

沿用强类型 camelCase DTO 与统一 `AppError`（`{ code, stage, operationId?, retryable, safeMessage, technicalDetail? }`），默认向界面呈现简洁中文 `safeMessage`，技术细节折叠于 `<details>`。

| 分组 | 核心命令 | 说明 |
|---|---|---|
| **工作区与设置** | `resume_workspace`, `open_workspace`, `get_app_settings`, `save_app_settings`, `get_hotkey`, `set_hotkey`, `pick_workspace_folder`, `pick_workspace_markdown` | 统一读写 `settings.json` 与工作区配置，快捷窗打开时切换工作区先自动落盘隐藏再切换 |
| **模型与 OV 配置** | `get_integrations_config`, `save_model_config`, `test_model_connection`, `list_remote_models`, `save_memory_config`, `test_memory_connection` | `MemoryConfig` 与保存参数支持独立的 `syncResources`、`commitMemories`，旧配置缺省值均为 `true`。密钥走 Windows 凭据管理器，OV 凭据可以使用 `Bearer ` 前缀 |
| **快记与草稿** | `show_capture_window`, `load_capture_workspace`, `persist_draft`, `commit_capture`, `confirm_capture`, `close_capture_window` | `commit_capture` 原子追加当日文件并更新索引，按全局自动偏好与工作区配置静默入队。队列优先级为 `ov_write_resource` → `organize` → `ov_commit_memory` |
| **记录与检索** | `list_recent_captures`, `search_records`, `rebuild_search_index`, `open_record_source`, `open_workspace_file`, `list_notes`, `get_note_detail`, `read_workspace_image` | 详情含 `ovResourceStatus`、`ovResourceUri`、`ovMemoryStatus`、`ovError`，两路状态为 `pending`、`synced` 或 `failed` |
| **后台整合与待办** | `trigger_note_organize`, `list_tasks`, `toggle_task`, `retry_memory_sync` | `retry_memory_sync` 接受可选的 `noteId`（记录 ID 或定位符），缺省扫描工作区记录并按文件合并原文同步，返回新增入队任务数。后台进度通过 `note-updated` / `drawer-badge-updated` 推送 |
| **冲突与 Wiki 抽屉** | `drawer_counts`, `list_conflicts`, `create_conflict`, `resolve_conflict`（`keep_current \| accept_incoming \| keep_both`）, `wiki_targets`, `map_wiki_target`, `unmap_wiki_target`, `create_wiki_target`, `list_wiki_suggestions`, `create_wiki_suggestion`, `apply_wiki_suggestion`, `recompare_wiki_suggestion`, `reject_wiki_suggestion`, `undo_wiki_suggestion` | Git 冲突三选一处理；支持解除 Wiki 映射、行级 Diff 预览与防覆盖重比较；Wiki 建议列表不再混入任务勾选记录 |
| **多轮记忆对话** | `list_conversations`, `get_conversation`, `delete_conversation`, `send_chat_message`, `read_memory_item` | 引用按 `local`、`ov_memory`、`ov_resource` 区分，`path` 对 OV 引用保存 Viking URI，同工作区同日原文命中优先保留本地。助手消息可含降级 `notice`。流式事件为 `chat-chunk` / `chat-done`，远端预览返回 `uri`、`kind`、`title`、`content` |
