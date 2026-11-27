# Wikra P0 前后端冻结接口契约与验收标准

- **产品名称**：**Wikra**（Windows 本地优先个人 LLM Wiki 工作台，100% 兼容 Obsidian 本地 Markdown 文件）
- **阶段与状态**：**P0（真·多页增量编译引擎 + 混合 Lint + 本地只读 `stdio` MCP / `SKILL.md` + 统一 `[[` 双链补全）FROZEN（已冻结）** · 2026-11-27
- **仓库主干**：`D:/Development/flash-marking-down`（`main` 基线提交 `46a06e4`）
- **上游依据**：[P0 三轨方案比对与统一定稿（D1–D5 全选 A）](P0-THREE-WAY-SYNTHESIS.md) · [P0 增量 UI 设计规格书](ui-design/P0-UI-SPEC.md) · [Wikra v2.0 冻结契约](INTERFACE-CONTRACTS-AND-ACCEPTANCE-v2.md)

---

## 1. 已拍板决策与工程红线（不可违背）

1. **已拍板 5 项决策（D1–D5 全选 A）**：
   - **D1-A**：`package.json` 与 `package-lock.json` 根节已由总控显式声明锁定在 lockfile 中的 `"@codemirror/autocomplete": "6.20.4"`（零新下载、零版本变化）；前端与后端实施方均**严禁**修改 `package.json`、`package-lock.json`、`Cargo.lock`（Rust 侧仅允许在 `src-tauri/Cargo.toml` 增加 `default-run = "wikra"` 与 `[[bin]] name = "wikra-mcp"`，**零新增外部 Crate**）。
   - **D2-A**：分层自治默认值——普通新页（达 claim 门槛）、保护区外追加、补双链、补经证实且无歧义别名自动落盘（可整批或按页安全撤销）；`replace_section`、决策页/核心页、冲突或不确定、薄新页、同名 H2 不唯一、写盘前 `baseHash` 变化、单批变化内容页 >15 页一律进待审；保护区（`<!-- wikra-protect:start/end -->` 与旧 `fmd-protect`）永不写入。
   - **D3-A**：MCP 两级隐私——默认只暴露 `wiki/`，`get_evidence` 默认只返回 Wiki 页已有逐字短引及本机核验态 `verified`；在设置中按文件夹显式放开 `raw/` 或随手记子目录后，外部 Agent 才可检索/读取该目录并在 `get_evidence` 中获得原段前后各 1 段（硬上限 1,200 字）。`.wikra/`、草稿、凭据永远不可访问。
   - **D4-A**：新页文件名固定为 `wiki/<类型目录>/<标题>.md`（去除 Windows 非法字符，重名加 ` (2)`），确保 Obsidian `[[标题]]` 原生可解析。
   - **D5-A**：`wiki/index.md` 维护 `<!-- wikra-index:start -->` ~ `<!-- wikra-index:end -->` 托管块（不存在则新建，已有无标记则在文末追加，块外人工内容逐字保留，声明 `wikra-managed: false` 则跳过）。
2. **五条务实工程红线**：路径不越界（排除 `.wikra` / `.flash-marking-down`）、写入原子替换且正文快照**仅存一份**（不做 2PC、不做父目录锁、不做 NTFS 绑定、不建向量库）、快记不丢字且严守 IME 四级 `Esc`、凭据仅存 Windows 凭据管理器、预览防 XSS。
3. **Obsidian 往返保真**：局部改写 Frontmatter / 章节 / 追加链接时保留 BOM、主换行符（CRLF/LF）、未知 Frontmatter 键与键序、缩进、插件块与注释 `%%...%%`；`CaptureEditor.vue` 必须保留 CRLF 换行不静默转 LF。
4. **P1/P2 预留位（只透传不实现业务）**：Frontmatter 透传保留 `status` / `supersedes` / `superseded_by` / `agent_export`；`set_frontmatter_key` 白名单**不含**这些键，L1 不对其报错，MCP `read_page` 原样透传。

---

## 2. SQLite Schema v7 → v8 迁移契约（`src-tauri/src/storage.rs` & `storage/wikra.rs`）

`SCHEMA_VERSION` 升级至 `8`。单事务无损迁移（兼容历史 v1–v7 工作区自动链式升级至 v8，MCP 只读进程**绝不**执行迁移）：

```sql
-- 1. 扩展 wikra_changesets 主表（status 取值：applied | partial | reverted | partially_reverted）
ALTER TABLE wikra_changesets ADD COLUMN parent_id TEXT REFERENCES wikra_changesets(id);
ALTER TABLE wikra_changesets ADD COLUMN note TEXT NOT NULL DEFAULT '';
ALTER TABLE wikra_changesets ADD COLUMN dropped_claims INTEGER NOT NULL DEFAULT 0;

-- 2. 新增多页快照子表（每页仅存一份写前快照）
CREATE TABLE IF NOT EXISTS wikra_changeset_pages (
    changeset_id TEXT NOT NULL REFERENCES wikra_changesets(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    title TEXT NOT NULL,
    before_snapshot TEXT,                        -- NULL 表示本批新建页
    after_hash TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('applied', 'reverted', 'kept_modified')),
    PRIMARY KEY (changeset_id, path)
);

-- 3. 新增结构化操作子表（待审 op 仅存 payload_json + base_hash，绝不存整页正文）
CREATE TABLE IF NOT EXISTS wikra_changeset_ops (
    changeset_id TEXT NOT NULL REFERENCES wikra_changesets(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    op TEXT NOT NULL,                            -- create_page | append_section | replace_section | add_link | set_frontmatter_key
    page_path TEXT NOT NULL,
    page_title TEXT NOT NULL,
    label TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    base_hash TEXT,
    outcome TEXT NOT NULL CHECK(outcome IN ('applied', 'pending_review', 'discarded', 'dropped')),
    reason TEXT NOT NULL DEFAULT '',
    conflict_id TEXT REFERENCES conflicts(id),
    child_changeset_id TEXT REFERENCES wikra_changesets(id),
    PRIMARY KEY (changeset_id, seq)
);
CREATE INDEX IF NOT EXISTS wikra_changeset_ops_pending ON wikra_changeset_ops(outcome, page_path);

-- 4. 新增 L2 定向语义 Lint 缓存表（L1 实时计算不落库；静音复用 dismissed_suggestions，id 前缀 'lint:'）
CREATE TABLE IF NOT EXISTS wikra_lint_findings (
    id TEXT PRIMARY KEY,
    rule TEXT NOT NULL,                          -- semantic_duplicate | semantic_contradiction
    paths_json TEXT NOT NULL,
    page_hashes_json TEXT NOT NULL,
    detail_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- 5. 无损回填历史 v7 单页记录并清空旧表重复快照（红线：正文快照只保留一份）
INSERT OR IGNORE INTO wikra_changeset_pages (changeset_id, path, title, before_snapshot, after_hash, state)
SELECT id, target_path, target_title, before_snapshot, after_hash,
       CASE status WHEN 'reverted' THEN 'reverted' ELSE 'applied' END
FROM wikra_changesets;

INSERT OR IGNORE INTO wikra_changeset_ops (changeset_id, seq, op, page_path, page_title, label, payload_json, outcome)
SELECT id, 1,
       CASE WHEN before_snapshot IS NULL THEN 'create_page' ELSE 'append_section' END,
       target_path, target_title, op_label, '{}',
       CASE status WHEN 'reverted' THEN 'discarded' ELSE 'applied' END
FROM wikra_changesets;

UPDATE wikra_changesets SET before_snapshot = NULL;
```

- MCP 允许读取的 `raw/` 子目录白名单持久化于既有 `workspace_settings` 表键 `'mcp_raw_allow'`（JSON 字符串数组，默认 `[]`）。

---

## 3. 后端核心算法与行为契约（M1–M3）

### 3.1 M1 真·多页增量编译引擎（`src-tauri/src/storage/wikra/organize/`）

1. **本地 4 路召回（`route.rs`，0 Token）**：
   - 仅检索 `wiki/` 下非 `meta()`（`index.md` / `log.md` / `SCHEMA.md` / `SKILL.md`）、非 `wikra-managed: false` 的知识页；meta 页出链**不计入**入度、孤页、Hub 与路由邻域权重。
   - 信号打分：① 原文显式 `[[...]]` 唯一命中 `+100`；② 标题 `+40` / 别名 `+30`（中文 ≥2 字符、ASCII ≥3 字符带词边界，重名跳过）；③ FTS5 trigram `bm25`（仅对 ≥3 字符的 ≤8 个查询词使用 `fts_trigram_phrase`，`+10..+35`）；④ Top-5 候选沿图谱 1 跳出链/反链扩展 `+15`。
   - 构造紧凑 `index_map`（**≤12 行**：`cN | path | title | aliases | docType | summary(≤60字)`）。`SCHEMA.md` ≤4KB 全量送入，超出按全局 + 涉及页型 H2 切片。素材单批上限 12,000 字符，超出段落保留未编译状态供下次续编。
2. **自适应单步/两步协议（`edit_ops.rs`）**：
   - **合并为 1 次请求**：空库（0 篇非 meta Wiki 页），或（候选 ≤3 且无 `decision`/`wikra-core` 页、无重名且估算输入 ≤16k 字符）。
   - **拆分为 2 次请求**：Step 1 输出 `{ noop, noopReason, routes: [{ units, target, relation, why }] }`（`relation ∈ extends | duplicate | different_scope | conflicts | uncertain`，已有目标 ≤4，新页 ≤2，新页标题撞名自动转指向已有页）；Step 2 仅接收短引用号 `p1..p4 / h1..hM / s1..sN / n1..n2`，输出扁平 `{ summary, ops: [...] }`（5 种 op：`create_page` / `append_section` / `replace_section` / `add_link` / `set_frontmatter_key`）。
   - 复用 `model::generate_with`（代码围栏剥离 + 1 次格式重试预算）。
3. **短引核验与双链放行**：
   - 每条 claim 逐条核验：先在本批对应原段（`sN`）做精确子串匹配；失败则做空白折叠匹配并**回映取出原文真实单行子串**（跨行判失败，超 240 字判失败）。单条失败仅丢弃该 claim（累加 `dropped_claims`），依赖它的空页/链一并移除；`replace_section` 任一 claim 失败则整条 `replace_section` 丢弃、保留旧节；全批有效 op = 0 且非 `noop`/`duplicate` 才报错重试。
   - 正文文本不再把 `[`/`]` 转义为 `&#91;`/`&#93;`：仅当 `[[X]]` 中的 `X` 在现有库中唯一解析或等于本批新页标题时保留真双链，否则去方括号保留文字；同步剥离模型伪造的 `[^...]`。
4. **分层自治与 `MarkdownBatch` 落盘（`apply.rs`）**：
   - **Drop**：目标非 `wiki/` 或为 meta/`raw_dir`/`capture_dir`/状态目录；`wikra-managed: false`；存在未闭合保护标记/代码围栏；写区与 `wikra-protect`（含 `fmd-protect`）相交；`set_frontmatter_key` 的 key 非 `aliases` 或别名不在本批原段中逐字出现或撞名；剩余 claim 为 0。
   - **Review（`pending_review`）**：`replace_section`；触及 `wiki/decisions/`、`type: decision` 或 `wikra-core: true`；新建决策页；route 关系为 `conflicts`/`uncertain`（同步创建一条 `conflicts` 记录关联 `conflict_id`，**不往 Markdown 文件写 `wikra_pending_conflicts`**）；薄新页（非 entity/decision 且有效 claim <2）；同名 H2 不唯一；写盘前 `baseHash` 变动；本批变化内容页 >15 页（整批转待审）。
   - **Auto（`applied`）**：其余普通新页、保护区外追加、`add_link`（写入 `## 相关链接`，幂等去重）、`aliases` 局部并集追加。
   - `MarkdownBatch::commit`：内存算齐 `after` → 开 `IMMEDIATE` 事务写 DB 与 FTS → 顺序原子替换文件（失败则逆序恢复已写且哈希匹配的文件并回滚事务）→ 提交事务 → 更新 `wiki/index.md`（>200 页每组限最近 30 条 + 其余 N 页提示）与只追加 `wiki/log.md`（簿记失败仅告警不撤销）。
   - **按页确认待审（`decide_review`）**：按 `(changesetId, path)` 整页采纳或丢弃；采纳前硬校验磁盘当前哈希 `=== base_hash`，不匹配返回 `VERSION_CONFLICT`；通过后生成带 `parent_id` 的子 Changeset 并回填父 op `child_changeset_id`，关闭已清零关联冲突。**严禁**调用旧 `resolve_conflict` 触发 `create_wiki_suggestion` 副作用。
   - **按页部分回滚（`revert_changeset`）**：若存在未撤销子批次则返回错误提示先撤子批次；逐页核对磁盘哈希 `=== after_hash` → 匹配则还原 `before_snapshot`（新建页删除）标 `reverted`，不匹配则保留人工内容标 `kept_modified`；未处理的 `pending_review` 置 `discarded`；仅当本批全部关联页撤销后才复位 `inbox_paragraphs.compiled = 0`；重算 `wiki/index.md` 并追加 `[revert]` 到 `wiki/log.md`。

### 3.2 M2 混合 Lint 引擎（`src-tauri/src/storage/wikra/lint.rs`）

- **L1（9 条规则，Rust 0-Token 实时计算）**：
  - `broken_link`（`error`，唯一候选时 `fix = { kind: 'one_click', label: '一键改链' }`，走 Changeset）、`missing_source` / `quote_mismatch`（`error`，不自动修）、`structure_blocked`（`error`，不自动修）；
  - `broken_anchor`（`warn`，覆盖全级标题 + `^id` + `[[#本页标题]]`）、`ambiguous_name`（`warn`）、`stale_conflict_marker`（`warn`，`fix = { kind: 'safe', label: '清理标记' }`，走 Changeset）；
  - `index_drift`（`info`，全库聚合 1 条，`fix = { kind: 'safe', label: '同步目录' }`，直接重建托管块）、`orphan`（`info`，剔除 meta 边且豁免 24h 内新页）、`thin_page`（`info`，正文 <80 字且无脚注）。
  - 排除 `%%...%%` 注释、代码围栏与行内代码；按内容指纹（不含行号）匹配 `dismissed_suggestions`（前缀 `lint:`），在 `LintFindingDto` 中输出 `dismissed: boolean` 以支撑三态筛选与 `undismiss_suggestion`。
- **L2（定向语义 Lint，仅手动触发）**：
  - 范围：`{ kind: 'page', path }`（当前页 + 1 跳邻域） / `{ kind: 'recent', days }`（默认 7 天） / `{ kind: 'folder', path }`；硬上限 **12 页、6 候选对**。
  - 本地预筛（标题 trigram 相似度 ≥0.6 / 别名交集 / 同源段落 / 互链且同 H2 词）；模型输出 `semantic_duplicate` / `semantic_contradiction` 必须附带 `quoteA` + `quoteB` 且经 Rust 双侧原文逐字硬核验，`low` 丢弃，单次最多 5 条，按页哈希缓存于 `wikra_lint_findings`。

### 3.3 M3 独立控制台二进制 `wikra-mcp.exe` + `SKILL.md`

- `src-tauri/Cargo.toml` 增加 `default-run = "wikra"` 及 `[[bin]] name = "wikra-mcp" path = "src/bin/wikra_mcp.rs"`；`src-tauri/src/lib.rs` 导出 `pub mod mcp`。
- `wikra-mcp.exe --vault <绝对路径>`（兼容 `WIKRA_VAULT`）：走独立只读路径 `WorkspaceFiles::open_read_only(root)`（**绝不建目录/不迁库/不调 `ensure_index_fresh`**）；SQLite 用 `SQLITE_OPEN_READ_ONLY` + `PRAGMA query_only = ON; busy_timeout = 2000;`，核对 `application_id` 与 `user_version == 8`，不满足或无库时降级纯文件模式（`indexState: "files_only"`）。Catalog 按 `(文件数, 最大 mtime, 总大小)` 签名进程内缓存。
- NDJSON（单行 JSON-RPC 2.0 over `stdio`），stdout 仅协议消息，stderr 仅脱敏诊断；协议版本支持 `["2025-11-25", "2025-06-18", "2025-03-26"]` 协商。
- **3 工具（同时返回 `content` 文本 + `structuredContent`，`read_page` 文本中警告置于正文之前）**：
  - `search({ query, type?, folder?, limit? /*1..20, 默认8*/ })` → `{ hits: [{ path, title, type, snippet, pendingConflictCount }], indexState }`
  - `read_page({ path?, title?, maxChars? /*默认20000*/ })` → `{ path, title, frontmatter, body, truncated, warnings, pendingConflicts: [{ id, topic }], citations: [{ citationRef, label, rawPath, quote }], backlinks }`
  - `get_evidence({ citationRef })` → `{ citationRef, rawPath, quote, verified, context: "quote" | "paragraph", paragraph: string | null, reason?: "raw 未授权" | "原文缺失" }`
- **2 Resources**：`wikra://vault/schema`、`wikra://vault/index`（托管块每组前 30 条）。
- **`SKILL.md`**：Vault 根写托管块 `<!-- wikra-skill:start/end -->`，固定 `name: wikra-lookup`，校验 `expectedHash` 且内容未变零写入；纳入 `Document::meta()`。

---

## 4. 冻结 Tauri IPC 命令与 TypeScript / Rust DTO 契约（camelCase JSON + 统一 `AppError`）

除 `list_wikilink_candidates` 允许主窗与活动快捷窗调用外，其余新增命令均限定主窗调用。

### 4.1 既有 DTO 兼容扩展

```ts
// 1. LibraryDocSummary 扩展
export type LibraryDocSummary = {
  // ...既有字段不变...
  aliases: string[]
  status?: string | null       // P1 预留透传
}

// 2. DocumentDetail 扩展
export type DocumentDetail = {
  // ...既有字段不变...
  status?: string | null       // P1 预留透传
  supersedes?: string[]        // P1 预留透传
}

// 3. ChangesetOp / ChangesetItem 扩展（100% 向下兼容既有字段）
export type ChangesetPageState = 'applied' | 'reverted' | 'kept_modified'
export type ChangesetPage = {
  path: string
  title: string
  state: ChangesetPageState
}
export type ChangesetOpOutcome = 'applied' | 'pending_review' | 'discarded' | 'dropped' | 'reverted'
export type ChangesetOp = {
  seq: number
  op: 'create_page' | 'append_section' | 'replace_section' | 'add_link' | 'set_frontmatter_key' | string
  label: string
  targetPath: string
  targetTitle: string
  outcome: ChangesetOpOutcome
  reason: string
  conflictId: string | null
  childChangesetId: string | null
}
export type ChangesetItem = {
  id: string
  time: string
  trigger: string
  modelAlias: string
  status: 'applied' | 'partial' | 'reverted' | 'partially_reverted' | string
  summary: string
  note: string
  parentId: string | null
  droppedClaims: number
  pagePaths: string[]
  pages: ChangesetPage[]
  ops: ChangesetOp[]
}

// 4. InboxItemDetail 扩展可选整理摘要
export type CompileSummary = {
  changesetId: string
  applied: number
  pending: number
  dropped: number
  remainingParagraphs: number
}
export type InboxItemDetail = InboxItemSummary & {
  summaryPoints: string[]
  relatedPagePaths: string[]
  paragraphs: InboxParagraph[]
  compileSummary?: CompileSummary | null
}
```

### 4.2 新增与升级 IPC 命令全清单（共 16 项）

| # | 命令名 | 入参（Rust snake_case / TS camelCase） | 返回 DTO |
|---|---|---|---|
| 1 | `get_changeset_detail` [新] | `(session, changesetId)` | `ChangesetDetail { changeset: ChangesetItem, pages: ChangesetPageDiff[] }`，每项 `{ path, title, state, hasPending, pendingReasons: string[], hasProtectedSection, before, after, current, externallyModified }` |
| 2 | `decide_review` [新] | `(session, changesetId, path, decision: 'apply' \| 'discard')` | `ChangesetItem`（`apply` 前硬校验磁盘哈希 `=== base_hash`，不符返回 `VERSION_CONFLICT`） |
| 3 | `revert_changeset` [升级] | `(session, changesetId)` | `RevertChangesetResult { changeset: ChangesetItem, restored: string[], keptModified: string[] }` |
| 4 | `run_structure_lint` [新] | `(session)` | `LintReport { generatedAt, pageCount, unreadable, durationMs, findings: LintFinding[] }` |
| 5 | `apply_lint_fix` [新] | `(session, findingIds: string[])` | `LintFixResult { changeset: ChangesetItem \| null, indexRebuilt: boolean, report: LintReport }` |
| 6 | `undismiss_suggestion` [新] | `(session, suggestionId: string)` | `()`（从 `dismissed_suggestions` 删除指定 id，支持体检「已忽略」恢复提示） |
| 7 | `preview_semantic_lint` [新] | `(session, scope: SemanticLintScope)` | `SemanticLintPreview { pages: string[], pageCount: number, pairCount: number, estimatedChars: number, modelAlias: string }` |
| 8 | `run_semantic_lint` [新] | `(session, scope: SemanticLintScope)` | `LintFinding[]`（仅返回本次 L2 开放告警，且写入缓存供合并展示） |
| 9 | `escalate_lint_to_conflict` [新] | `(session, findingId: string)` | `ConflictItem`（将 L2 矛盾告警转为开放冲突，不触发旧 `create_wiki_suggestion`） |
| 10 | `list_wikilink_candidates` [新·双窗] | `(session)` | `WikilinkCandidate[]`，每项 `{ path, stem, title, aliases: string[], folder, typeLabel, linkText, ambiguousStem: boolean }` |
| 11 | `resolve_wikilink` [新] | `(session, fromPath: string, target: string)` | `WikilinkResolution { status: 'unique' \| 'ambiguous' \| 'missing', matches: { path, title, typeLabel, updatedAt }[] }` |
| 12 | `read_citation_context` [新] | `(session, pagePath: string, refLabel: string)` | `CitationContext { rawPath, quote, verified, paragraph, prev, next, reason: string \| null }` |
| 13 | `get_agent_integration` [新] | `(session)` | `AgentIntegration { mcpExePath, mcpExeExists, vaultRoot, rawDir, rawDirs: RawDirOption[], rawAllow: string[], snippets: { claudeCli, mcpJson, codexToml }, skillPath, skillExists }` |
| 14 | `set_mcp_raw_allow` [新] | `(session, folders: string[])` | `string[]`（边界校验仅允许 `raw_dir` / `capture_dir` 及其工作区内子目录） |
| 15 | `export_vault_skill` [新] | `(session)` | `ExportSkillResult { path, action: 'created' \| 'updated' \| 'unchanged' }` |
| 16 | `test_mcp_stdio` [新] | `(session)` | `McpTestResult { ok: boolean, protocolVersion: string \| null, tools: string[], error: string \| null }` |

附加类型精确定义：
```ts
export type SemanticLintScope =
  | { kind: 'page'; path: string }
  | { kind: 'recent'; days: number }
  | { kind: 'folder'; path: string }

export type LintFindingFix = {
  kind: 'safe' | 'one_click'
  label: string
}

export type LintFinding = {
  id: string                  // "lint:<fingerprint>"
  layer: 'L1' | 'L2'
  rule: string
  severity: 'error' | 'warn' | 'info'
  pagePath: string
  pageTitle: string
  line: number | null
  relatedPaths: string[]
  message: string
  dismissed: boolean
  fix: LintFindingFix | null
  evidence: { quoteA: string; quoteB: string; pathA: string; pathB: string; titleA: string; titleB: string } | null
}

export type RawDirOption = {
  path: string
  fileCount: number
  isCaptureDir: boolean
}
```

- **补全与模糊匹配约定（M4）**：
  - `CaptureEditor.vue` 使用 `@codemirror/autocomplete`（`6.20.4`）构建统一 `src/editor/wikilinkComplete.ts`，支持 `[[` 与中文 `【【`（选中后统一替换为标准 `[[...]]`，并吸收光标后紧邻的 `】】` / `]]`）。
  - 快捷窗使用 `tooltips({ position: 'fixed' })` 且弹层最大高度 `min(220px, calc(100vh - 110px))`，540×380 窗口内绝不裁切；严守四级 `Esc` 捕获早退链（IME 组词 → 补全打开时裸 `Esc/Enter/Tab/↑/↓` 早退交 CM6，`Ctrl+Enter` / `Ctrl+Shift+Enter` 照常穿透 → 空草稿关窗 → 非空落盘关窗）。
  - `CaptureEditor.vue` 保留文件主换行符（`EditorState.lineSeparator` + `state.sliceDoc()`），禁止将 CRLF 文件静默改写为 LF。
  - 删除 `CaptureWindow.vue` 旧文末正则补全与 `WIKRA_WIKI_CACHE_KEY` 跨库 `localStorage` 缓存，两窗统一按当前 `session` 调用 `list_wikilink_candidates`。

---

## 5. 实施分工与验收门禁

- **后端（Codex `gpt-6.2-sol`, `max`）**：仅修改 `src-tauri/`（严禁改动 `src/`、`package.json`、`package-lock.json`、`Cargo.lock`、`docs/`，严禁 `git commit`/`git push`）。交付完整 M1 + M2 + M3 + 后端 M4 支撑命令与单元测试，完成 `cargo fmt --check`、`cargo test --locked`、`cargo build --bin wikra-mcp --locked`。
- **前端（内置子代理 `wikra_frontend_agent`）**：仅修改 `src/`（严禁改动 `src-tauri/`、`package.json`、`package-lock.json`、`docs/`，严禁 `git commit`/`git push`）。严格对照本冻结契约与 [P0-UI-SPEC.md](ui-design/P0-UI-SPEC.md) 交付全部 M1/M2/M3/M4 UI、白话文案、双主题样式、CRLF 保真与 IME 四级 `Esc`，完成 `npm run build` 零报错。
- **总控验收门禁（开发完毕后全量执行）**：
  1. 逐文件审查前后端 Diff，修复任何字段/语义对齐偏差；
  2. 全量通过 `npm run build`、`cargo fmt --manifest-path src-tauri/Cargo.toml --check`、`cargo test --manifest-path src-tauri/Cargo.toml --locked`、`cargo build --manifest-path src-tauri/Cargo.toml --bin wikra-mcp --locked`、`npm run tauri build -- --debug --no-bundle -- --locked`；
  3. 实测 `wikra-mcp.exe` 真实 `stdio` 管道 JSON-RPC 2.0 握手、`tools/list`、`search`/`read_page`/`get_evidence` 及默认 `raw/` 隐私隔离；
  4. 经本机 Magpie 网关用纯合成测试样本验证真实多页编译（A1–A4）并在隔离临时 Vault 核验回滚与 Obsidian 往返保真。
