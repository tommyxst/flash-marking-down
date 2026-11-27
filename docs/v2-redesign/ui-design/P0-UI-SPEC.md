# Wikra P0 增量交互设计稿与文案规范（P0-UI-SPEC）

- **版本**：P0-UI-SPEC Rev.1.1（基于 Rev.3 设计系统增量演进，已落实交互与文案审查修订）
- **日期**：2026-11-27
- **上游依据**：[P0 三轨方案比对与统一定稿（D1–D5 全选 A）](../P0-THREE-WAY-SYNTHESIS.md) · [Wikra v2.0 UI 设计规格书 Rev.3](UI-DESIGN-SPEC-v2.md)
- **可交互高保真原型**：[Wikra-P0-Preview.html](Wikra-P0-Preview.html)

---

## 1. 设计基线与红线约束

1. **视觉系统 100% 沿用 Rev.3**：复用 [styles.css](styles.css) 的双主题 CSS 变量——**Theme A（Cognitive Slate 深色默认）** 与 **Theme C（Paper Daylight 浅色切换）**，本地离线字体 `./fonts/hanken-grotesk.woff2` 与 `./fonts/jetbrains-mono.woff2`（中文回落 `Microsoft YaHei UI`）。
2. **严格视觉克制**：零外链 CDN、零装饰渐变、零 Emoji、零 AI 发光球、零晦涩工程/治理黑话。
3. **不破坏既有信息架构**：保留左侧 56px 主导航（知识库 / 收件箱 / 整理 / 图谱 + 底栏主题与设置），所有 P0 新能力以增量方式融入既有页面，不新增顶层导航项。
4. **宽屏与高 DPI 窄视口双端适配**：
   - `1600px–1920px` 宽屏：「整理」页沿用 `max-width: 1680px` 双栏并排，待审差异面板支持 50/50 并排 Diff；
   - `1100×700` 窄视口（覆盖 1080p @ 125%–150% DPI）：主舞台扣除 56px 左侧导航后宽约 `1044px`，高于 `@container stage (max-width: 860px)` 单栏阈值，因此整理页仍保持双栏（单栏约 `450px–515px`）。为保证窄栏零溢出，所有卡片头与行级操作（`.ops-card-head`、`.ops-row`、`.lint-row`、`.cs-op-row`、`.skill-path-row`）必须配置 `flex-wrap: wrap`，长路径与代码标识配置 `word-break: break-all`；当主舞台缩窄至 `≤ 860px`（如小窗口或展开侧栏）时整理页折为单栏，Diff 面板在容器宽度 `< 840px` 时折为上下对照；知识页消歧浮层与引文悬停浮窗启用水平边界钳位（距容器左右边缘至少留 `12px`）；随手记浮窗严格适配真实尺寸 **`540×380`**（`minWidth: 420`、`minHeight: 280`），补全弹层边缘钳位不溢出且不遮挡底栏；全程零横向滚动条。

---

## 2. 文案净化与白话映射规范

界面禁止向用户直接暴露数据库字段、内部协议名或工程枚举（`Changeset`、`EditOp`、`pending_review`、`VERSION_CONFLICT`、`L1/L2`、`kept_modified`、`thin_page` 等），统一采用下表白话表达：

| 后端/协议概念 | 界面白话文案 | 使用场景 |
|---|---|---|
| `Changeset` | `整理记录` / `本次整理` / `整理批次` | 整理页右栏卡片与提示 |
| `status: applied` | `已完成`（`pill.success`） | 整理记录状态胶囊 |
| `status: partial` | `含待确认项`（`pill.warning`） | 整理记录状态胶囊 |
| `status: partially_reverted` | `已部分撤销`（`pill.info`） | 部分回滚后的状态胶囊 |
| `status: reverted` | `已撤销`（中性 `pill`） | 全量回滚后的状态胶囊 |
| `op: create_page` | `新建` | 多页操作行前缀胶囊 |
| `op: append_section` | `追加` | 多页操作行前缀胶囊 |
| `op: replace_section` | `替换章节` | 多页操作行前缀胶囊 |
| `op: add_link` | `补双链` | 多页操作行前缀胶囊 |
| `op: set_frontmatter_key (aliases)` | `补别名` | 多页操作行前缀胶囊 |
| `outcome: applied` | `已写入` | 操作结果 |
| `outcome: pending_review` | `等待你确认` | 操作结果（点击可开 Diff） |
| `outcome: discarded` | `已丢弃` | 待审丢弃后状态 |
| `outcome: dropped` | `已跳过` | 保护区/短引校验未通过 |
| `page.state: reverted` | `已还原` | 回滚反馈 |
| `page.state: kept_modified` | `已保留你的修改` | 按页部分回滚反馈 |
| `VERSION_CONFLICT` | `页面刚刚有新变动，为防覆盖你的编辑，本次未写入` | 待审确认冲突拦截条 |
| `droppedClaims > 0` | `已跳过 N 条不符原文的结论` | 整理记录内温和提示行 |
| `L1 Structure Lint` | `知识库体检` | 整理页左栏卡片 ② 标题 |
| `L2 Semantic Lint` | `定向语义体检` | 体检卡底部触发区 |
| `mcp_raw_allow` | `允许外部 Agent 读取的原文目录` | 设置页第 6 卡片 |

### 2.1 知识库体检 9 条 L1 规则白话与交互映射

| 规则 ID | 级别 | 白话标签 | 说明文案模板 | 界面操作按钮 | 修复后行为 |
|---|---|---|---|---|---|
| `broken_link` | `error` | `断链` | 《{页}》引用了不存在的 `[[{目标}]]`（若有唯一候选：疑似指向《{候选}》） | 唯一候选：`一键改链`（仅行内逐条确认，不纳入批量修复）+ `打开页面` + `忽略`；无候选：`打开页面` + `忽略` | 改链走可撤销整理批次 |
| `missing_source` / `quote_mismatch` | `error` | `引文异常` | 《{页}》脚注 `[{ref}]` 对应的原文缺失或文字已变动 | **不提供自动修复**；仅 `打开页面核对` + `忽略` | — |
| `structure_blocked` | `error` | `标记未闭合` | 《{页}》存在未闭合的保护标记或代码块，AI 整理已跳过此页 | `打开页面修复` + `忽略` | — |
| `broken_anchor` | `warn` | `锚点失效` | 《{页}》链接指向的小节标题或块锚点不存在 | `打开页面` + `忽略` | — |
| `ambiguous_name` | `warn` | `重名链接` | 《{页}》中的 `[[{目标}]]` 同时匹配 {N} 篇同名知识页 | `打开页面` + `忽略` | — |
| `stale_conflict_marker` | `warn` | `残留冲突标记` | 《{页}》内残留已处理完毕的旧冲突标记 | `清理标记`（安全可修）+ `打开页面` + `忽略` | 清理走可撤销整理批次 |
| `index_drift` | `info` | `目录页待同步` | 全库聚合为 **1 张卡片**：`wiki/index.md` 与当前 {N} 篇知识页有 {M} 处出入 | `同步目录`（安全可修）+ `忽略` | 直接重新生成托管块，不进整理批次 |
| `orphan` | `info` | `尚未关联` | 《{页}》尚未与其他知识页互链（24 小时内新建页自动豁免） | `打开页面` + `忽略` | — |
| `thin_page` | `info` | `内容较单薄` | 《{页}》目前只有少量条目，可按需补充或并入相关页面 | `打开页面` + `忽略` | — |

---

## 3. 场景 1：「整理与同步」页增量（`OrganizeView.vue`）

延续 Rev.3 的两栏聚焦布局（`.organize-wrap` 最高 `1680px`，`.organize-cols` 左右 `1.15fr : 1fr`），不增加第三栏。

### 3.1 左栏卡片 ①：需要你确认的冲突与待审改动

- **合并呈现**：将现有「开放冲突（`conflicts`）」与多页整理产生的「待审改动（按 `changesetId + pagePath` 聚合的 `pending_review` ops）」统一收口在一张优先卡片中，彻底消除用户跨处寻找待办的负担。
- **头部状态**：存在任意待确认冲突或待审页时，卡片加 `.priority` 琥珀边框，标题旁显示合并计数胶囊（如 `1 处冲突 · 2 篇待审`）；全部清空后显示绿色 `已全部确认`。
- **行级结构**：
  - **观点冲突行**：左侧为 `观点冲突` 警告胶囊 + 主题标题 + 一句话差异说明；右侧操作按钮 **`左右对比确认`**。
  - **按页待审改动行**：左侧为 `待审改动` 强调胶囊 + 目标页面标题 + 触发待审的白话原因胶囊（如 `核心决策页` / `替换已有章节` / `单薄新页` / `来源变动`）+ 改动摘要（例如“替换「弱网退避策略」章节 · 含 2 条原文依据”）；右侧操作按钮 **`查看差异并确认`**。
- **并排 Diff 确认面板（点击「查看差异并确认」打开）**：
  - 对应后端 `get_changeset_detail`（按需现算 `proposed`，前端绝不提前缓存整页正文）与 `decide_review`（**按页**采纳或丢弃）。
  - **顶部信息栏**：页面标题、相对路径（monospace）、待审原因说明、以及一行保护区承诺提示（若页内含 `<!-- wikra-protect -->`：“你的「我的理解」手动保护区不受本次改动影响”）。
  - **并排差异区**：左栏「当前页面内容（磁盘现状）」，右栏「采纳后的页面效果」，删除/替换行以 `--error-soft` 标示，新增行以 `--success-soft` 标示，保留行号与清晰节标题；容器宽度 `< 840px` 自动改为上下排列。
  - **`VERSION_CONFLICT` 外部变动保护态**：当 `externallyModified === true` 或提交 `decide_review` 返回 `VERSION_CONFLICT` 时，在 Diff 顶部展示醒目提示条：
    - 文案：**「页面刚刚有新变动，为防覆盖你的编辑，本次未写入」** —— “该文件已被外部程序（如 Obsidian）或其他整理更新。请点击「重新检查文件状态」核对磁盘现状；仅当外部修改已还原回原基线时才会重新解锁「采纳写入」，否则请「打开页面手动合并」或「丢弃此页改动」。”
    - 动作：**「重新检查文件状态」**（重新调用 `get_changeset_detail` 刷新当前磁盘快照与 `externallyModified` 状态：若磁盘已还原回原基线则自动解除拦截并解锁「采纳写入」；若磁盘仍存在外部改动，则同步更新左栏「当前页面内容（磁盘现状）」并**保持「采纳写入」禁用**）；同时提供 **「打开页面手动合并」** + **「丢弃此页改动」** 双出口，绝不静默覆盖。
  - **底部决策条**：次按钮 **`丢弃此页改动`**、主按钮 **`采纳写入`**（冲突态下禁用，并提供 **`打开页面手动合并`** 入口）。采纳成功后按原 `op` 类型生成关联子整理批次（记录 `parentId`），右栏实时展示新生成的子批次并联动关闭已清零的相关冲突。

### 3.2 左栏卡片 ②：知识库体检（替代旧“可选的复习与关联建议”）

- **卡片头部**：
  - 左侧：图标 + `知识库体检` + 汇总胶囊 **`N 项错误 · M 项提醒`**（无错误且无提醒时显示绿色 `未发现异常`）。
  - 右侧：静默按钮 **`重新检查`** + 条件显示的主按钮 **`一键修复安全项 (K)`**（仅当存在 `fix.kind === 'safe'` 的未忽略项——即 `stale_conflict_marker`、`index_drift`——时出现，调用 `apply_lint_fix`）；`broken_link`（`fix.kind === 'one_click'`）涉及改写双链指向，不纳入批量一键修复，只能在行内逐条核对后点击 **`一键改链`**。
- **单页过滤上下文条（由知识页徽标跳转时出现）**：
  - 紧贴卡片头部下方展示轻量信息条：`正在查看《{页面标题}》的体检项` + 按钮 **`查看全库`**，一键恢复全库视角。
- **防疲劳筛选胶囊**：
  - 三态分段控件：**`需关注 (E+W)`**（默认选中，**仅展示 `error` 与 `warn`**）｜**`全部 (总计)`**（展开 `index_drift`、`orphan`、`thin_page` 等 `info` 项）｜**`已忽略 (D)`**（复用 `dismissed_suggestions`，前缀 `lint:`，提供 **`恢复提示`**；依赖冻结契约补齐 `LintFindingDto.dismissed: boolean` 与 `undismiss_suggestion(session, suggestionId)`）。
  - 在「需关注」态下，若有被折叠的 `info` 项，列表底部展示单行低视觉权重说明：“已折叠 {K} 项低优先级建议（如单薄页、未关联页），切至「全部」可查看”。
- **底部轻量「定向语义体检（L2）」触发区**：
  - 以细分割线收于体检卡底部，默认不打扰日常浏览。
  - **范围选择**：三选一分段/胶囊——**`当前页邻域`**｜**`近 7 天变动`**｜**`指定文件夹`**（存在当前打开的知识页上下文时允许选择「当前页邻域」；**无当前知识页时默认选中 `近 7 天变动`**，并将「当前页邻域」置灰且提示“请先打开一篇知识页”；选中文件夹时在旁出现紧凑下拉选择，如 `wiki/decisions/`）。
  - **两步确认防偷跑 Token**：
    1. 选择范围后自动或点击「预估开销」调用 `preview_semantic_lint`，在下方以单行紧凑信息条呈现：**`预计扫描 {pages} 篇页面 · 筛出 {pairs} 组候选对 · 约 {estimatedChars} 字 · 模型：{modelAlias}`**，附注“仅发送候选摘录，确认后才调用模型”。
    2. 用户显式点击 **`确认开始语义体检`** 后才调用 `run_semantic_lint`。若 `pairs === 0`，按钮置灰禁用并明确提示“当前范围内未筛出需要深度比对的相似页面”。
  - **L2 结果展示**：每条发现展示清晰问题描述 + **左右/上下双侧逐字短引**（两侧来源页面标题均为可点击双链，点击直接打开对应知识页），右侧提供 **`转为冲突`**（调用 `escalate_lint_to_conflict`，立即可见于上方卡片 ①）与 **`忽略`**。

### 3.3 右栏：待整理缓冲队列 + 升级版「AI 最近帮你整理了什么」

- **待整理缓冲队列**：沿用 Rev.3 `.buffer-banner`（有缓冲项时显示倒计时、`查看 / 撤回`、`立即整理`）。
- **升级版多页整理记录卡片**：
  - 卡片顶部：摘要标题 + 状态胶囊（`已完成` / `含待确认项` / `已部分撤销` / `已撤销`）+ 时间、触发来源与模型别名。
  - **子批次关联标识**：若 `parentId` 非空，在元信息行展示可点击微胶囊 **`由待审确认生成 · 关联 #{parentShortId}`**，点击可平滑滚动并短暂高亮父批次卡片。
  - **短引丢弃温和提示**：当 `droppedClaims > 0` 时，显示柔和信息行（`--muted` / `--info-soft`，不使用刺眼报错红）：`已跳过 {N} 条不符原文的结论（未写入知识库）`。
  - **多页操作列表（严格枚举驱动）**：每行由 `op` 枚举渲染固定左侧前缀胶囊（`新建` / `追加` / `替换章节` / `补双链` / `补别名`）+ 可点击页面双链 + 补充信息（如别名值、跳过原因）+ 右侧结果胶囊（`已写入` / `等待你确认` / `已丢弃` / `已跳过`）。其中被跳过的 `op`（`outcome: dropped`）**左侧保留原操作类型胶囊、右侧显示 `已跳过`**；`等待你确认` 行直接支持点击打开该页并排 Diff 面板。
  - **按页部分回滚与保护反馈**：
    - 点击 **`撤销此次整理`** 调用 `revert_changeset`：
      - 若存在未撤销的子批次，在卡片内**行内持久展示**拦截提示条（不随 Toast 消失，并附可点击的子批次定位按钮）：“请先撤销由本次待审确认生成的子整理记录（#{childId}），再撤销本批。”待对应子批次撤销后自动消除。
      - 回滚统计严格按**去重页面数**（`restored.length` / `keptModified.length`）计数，不按 `op` 条数重复累加；同时自动清理该批次下所有关联待审项。
      - 部分回滚成功（`restored.length > 0 && keptModified.length > 0`）时，卡片状态切换为 **`已部分撤销`**，并在卡片底部展示清晰反馈条（同时弹出轻量 Toast）：
        - 示例：**`已还原 2 篇；《API 超时与重试约定》因你后续手动修改，已原样保留。`**
      - 涉及的页面行同步更新最终状态胶囊（`已还原` vs `已保留你的修改`），绝不静默覆盖人工内容。

---

## 4. 场景 2：「设置 → 外部 Agent 接入」第 6 卡片（`SettingsModal.vue`）

在 `SettingsModal.vue` 既有 5 张设置卡片下方新增 **`6. 外部 Agent 接入（只读 MCP 与 SKILL.md）`**，视觉结构完全复用 `.setting-card`、`.field`、`.pill` 与 `.btn`。

### 4.1 顶部可执行文件状态与本地连接自检

- **第一行（组件探测）**：
  - 显示 `wikra-mcp.exe` 探测路径（只读 monospace 文本或紧凑路径条，启用 `word-break: break-all`）+ 状态胶囊：
    - 存在：`pill.success` **`已就绪`**；
    - 缺失：`pill.warning` **`未找到程序`** + 展开一行白话指引：“未在当前程序目录检测到 `wikra-mcp.exe`。请确认已将完整发布包中的 `wikra-mcp.exe` 与 `wikra.exe` 放在同一文件夹后，重新打开设置页面检查。”（禁止向最终用户展示报错或晦涩的构建命令行）。
  - 右侧按钮：**`测试本地连接`**（调用 `test_mcp_stdio`，仅执行 `initialize` + `tools/list` 握手；程序缺失时置灰）。
- **测试结果胶囊行**（点击测试后展示）：
  - 成功：`pill.success` **`握手正常 · 协议 2025-11-25 · 可用工具 3 个（search / read_page / get_evidence）`**；
  - 失败：`--error-soft` 提示条 + `pill.error` **`连接失败`**，明确展示可理解错误原因（如进程启动超时、被安全软件拦截或知识库目录不可读）。

### 4.2 多端接入配置片段与一键复制

- 三端切换标签（复用 `.segmented` 样式，严格对应后端 `snippets` 3 键契约 **`claudeCli` / `mcpJson` / `codexToml`**）：
  1. **`Claude Code CLI`**（`snippets.claudeCli`：一行 `claude mcp add ...` 命令，自动带入当前 Vault 绝对路径与 `wikra-mcp.exe` 路径）；
  2. **`Claude / Antigravity JSON`**（`snippets.mcpJson`：标准 `mcpServers` JSON 片段，注明适用于 Claude Desktop 与 Antigravity `mcp_config.json`）；
  3. **`Codex TOML`**（`snippets.codexToml`：`[mcp_servers.wikra]` 标准 TOML 配置块）。
- 代码框样式：`--inset` 背景、`--line` 边框、`JetBrains Mono 12px`、内部横向滚动（绝不撑宽弹窗），右上角固定 **`一键复制`** 按钮（复制后 2 秒内切为 `已复制`）。

### 4.3 `SKILL.md` 导出与安装指引（严格区分「已导出」与「已安装」）

- **导出状态行**：
  - 左侧：标题 `Agent 使用说明（SKILL.md）` + 状态胶囊——未生成时显示中性 `尚未导出`，生成后严格显示 `pill.info` **`已导出（Vault 根）`**（**严禁写成“已安装”**，避免误导用户以为外部工具已自动生效）。
  - 副标题：展示 Vault 根路径（如 `D:\Vault\MyBrain\SKILL.md`）并说明固定技能名为 `wikra-lookup`，写入采用托管块与哈希核验，不改动用户手写内容。
  - 右侧主/次按钮：**`生成 / 更新 Vault 根 SKILL.md`**（调用 `export_vault_skill`）。
- **安装到外部 Agent 目录的复制说明**（独立轻量提示盒）：
  - 白话文案：“生成到知识库根目录后，外部 Agent 不会自动扫描该文件。请按所用工具将其复制到对应技能目录（目录名需保持 `wikra-lookup`）：”
  - 列出常用目标路径并提供单行复制（路径行启用 `flex-wrap: wrap` + `word-break: break-all` 防窄视口溢出）：
    - Claude Code：`~/.claude/skills/wikra-lookup/SKILL.md`
    - Antigravity / 通用 Agent：`~/.agents/skills/wikra-lookup/SKILL.md`

### 4.4 原文目录授权区（D3-A 两级隐私模型）

- **顶部隐私边界说明条**：
  - 白话直述：**「默认仅暴露 `wiki/` 知识页与页内已有逐字短引，外部 Agent 看不到 `raw/` 与随手记正文。」**
  - 补充一句操作后果：“勾选下方目录后，外部 Agent 可检索、读取该目录内的原文，并在查证引文时获得原段前后各 1 段上下文（最多 1,200 字）。`.wikra/`、草稿与凭据永远不会暴露。”
- **按文件夹复选框列表**（来源 `get_agent_integration().rawDirs`，契约形态为 `{ path: string, fileCount: number, isCaptureDir: boolean }[]`，结合 `rawAllow: string[]` 渲染勾选态，仅列 `raw_dir` 与 `capture_dir` 下的子目录）：
  - 每项展示复选框 + 相对目录名（monospace，如 `raw/clips`）+ 篇数胶囊（`{fileCount} 篇`）；当 `isCaptureDir === true` 时标注低噪音提醒胶囊 **`随手记目录`** 与说明 `私人随手记 · 建议保持关闭`，普通原文子目录展示 `原文子目录 · 默认对外部 Agent 不可见`。
- **底部保存栏**：左侧动态显示当前放开汇总（如 `当前策略：仅知识页（未放开任何原文目录）` 或 `当前策略：知识页 + 已放开 1 个原文目录`），右侧按钮 **`保存目录授权`**（调用 `set_mcp_raw_allow`）。

---

## 5. 场景 3：编辑器 `[[` / `【【` 模糊补全浮层（`CaptureEditor.vue`）

依据已拍板的 **D1-A**，底层复用已锁定的 `@codemirror/autocomplete@6.20.4`，并统一接入 `list_wikilink_candidates`（主窗与随手记浮窗均可调用，废弃旧 localStorage 跨库缓存）。

### 5.1 触发与规范化规则

- **触发前缀**：在 CodeMirror 编辑器中输入 `[[` 或中文输入法（微软拼音全角符号）下的 **`【【`**，随后输入 0–40 个非换行非 `]`/`】` 字符时触发。
- **中文输入法静默守卫**：在 `composing || view.composing` 组词期间**禁止弹出补全框**，待候选词上屏（`compositionend`）后立刻匹配并弹出。
- **`【【` 自动规范化与闭合符吸收**：从补全框选中任意候选后，替换范围覆盖原始 `【【查询词`（或 `[[查询词`）并**吸收光标后紧邻的 `】】` / `]]`**，统一落盘标准 Obsidian 双链 `[[...]]`，绝不产生 `[[...]]]]` 或 `[[...]]】】`。
- **插入文本格式**（基础 `linkText` 由后端 `list_wikilink_candidates` 统一生成，别名命中时按目标路径规则拼接别名）：
  - 文件名唯一且与标题一致：`[[stem]]`
  - 文件名唯一且标题与文件名不同：`[[stem|标题]]`
  - 文件名唯一且通过别名命中选中：`[[stem|别名]]`
  - 同名文件重名（非别名命中）：`[[folder/stem|标题]]`
  - **同名文件重名 + 通过别名命中选中**：`[[folder/stem|alias]]`
- **单步撤销**：补全插入作为独立原子事务提交到 CodeMirror history，按一次 `Ctrl+Z` 立即完整回退到插入前的 `[[查询词`（或 `【【查询词`）。

### 5.2 补全弹层视觉与双场景边界适配

- **弹层尺寸与视觉**：
  - 宽度：`min-width: 264px; max-width: 344px`；
  - 高度与可见条目：
    - 主窗源码模式：最多显示 **6–8 项**（`max-height: 288px`，超出内部滚动，绝不撑开窗口）；
    - 随手记快捷窗（真实窗口尺寸 **`540×380`**，`minWidth: 420`、`minHeight: 280`）：CodeMirror 必须开启 **`tooltips({ position: 'fixed' })`** 避免被编辑器滚动区或窗体容器 `overflow` 裁切，并将弹层高度上限收紧为 **`max-height: min(220px, calc(100vh - 110px))`**（候选列表区内部滚动），确保在默认 `380px` 与最小 `280px` 高度下均完整可见且不遮挡顶栏与底栏；
  - 外观：`background: var(--surface-raised)`、`border: 1px solid var(--line-strong)`、`border-radius: 7px`、`box-shadow: var(--shadow-float)`，Slate / Paper 双主题自然适配。
- **候选行结构**（每行 `padding: 6px 9px`）：
  - 第一行：左侧 **页面标题**（命中关键词以 `--accent` 加粗高亮）+ 右侧 **类型胶囊**（`概念` / `决策` / `实体` / `主题`）；
  - 第二行（`11px`, `--muted`）：优先展示 **`别名命中：{alias}`**（通过别名召回时）或 **所在目录路径**（如 `wiki/decisions/`，便于一眼区分同名页）。
  - 键盘激活态（`aria-selected="true"`）：`background: var(--accent-soft)` + `border-color: var(--accent-border)`，文字高对比。
  - 底部固定单行微提示（`10.5px`, `--muted`）：`↑↓ 选择 · Enter / Tab 插入 · Esc 关闭`。
- **双场景定位契约**：
  1. **随手记浮窗（`CaptureWindow.vue`，窗口真实尺寸 `540×380`，`minWidth: 420` / `minHeight: 280`）**：开启 `tooltips({ position: 'fixed' })` 并设置高度上限 `min(220px, calc(100vh - 110px))`；默认紧贴光标下方 `4px` 展开；当光标靠近右侧导致 `left + tooltipWidth > 窗口宽度 - 8px` 时自动向左钳位（保留 `8px` 右边距）；当光标下方剩余高度不足以容纳弹层且不遮挡底栏时自动向上翻转展开，正常 / 右边缘钳位 / 底部上翻三态均完整可见且不遮挡底部操作栏。
  2. **主窗源码模式（`WikiDocumentView.vue` → `源码`）**：直接挂载在同一 `CaptureEditor.vue` 实例中，共享全部样式、候选源与快捷键规则。

### 5.3 四级 `Esc` 优先级与快捷键仲裁表（关键守卫）

随手记浮窗保持窗口级捕获阶段监听（`window.addEventListener('keydown', ..., true)`），按以下严格顺序仲裁；主窗保证前 2 级优先，第 3–4 级沿用主窗各视图原有逻辑：

| 优先级 | 判定条件 | 随手记浮窗（`CaptureWindow`）行为 | 主窗源码模式（`WikiDocumentView`）行为 |
|---|---|---|---|
| **第 1 级（最高）** | 处于中文 IME 组词态（`composing \|\| view.composing \|\| e.isComposing \|\| e.keyCode === 229`） | **直接 `return`**，绝不 `preventDefault`，全部按键（含 `Esc` / `Enter`）交还输入法 | 同左，直接 `return` 交还输入法 |
| **第 2 级** | 补全浮层已打开，且按下无修饰键的 `Esc` / `Enter` / `Tab` / `ArrowUp` / `ArrowDown` | **捕获阶段早退 `return`**，交由 CodeMirror `completionKeymap` 处理（`Esc` 仅关浮层、`Enter`/`Tab` 插入、`↑`/`↓` 选行） | 同左，交由 CodeMirror `completionKeymap` 处理 |
| **穿透特例** | 补全浮层已打开，但按下 `Ctrl+Enter` 或 `Ctrl+Shift+Enter` | **不早退**：正常触发「保存并整理」或「立即整理」，避免阻塞肌肉记忆 | 主窗 `Ctrl+S` 正常保存源码 |
| **第 3 级** | 补全浮层未打开，且草稿正文为空 | 阻止冒泡，立即收起浮窗 | 按主窗当前视图默认行为处理（如关闭弹窗） |
| **第 4 级** | 补全浮层未打开，且草稿正文非空 | 先落盘草稿再收起浮窗，**绝不静默提交**；若落盘失败保持可编辑，高亮「复制全文」与「暂存内存并收起」逃生出口 | 保持源码编辑态，不丢失未保存改动 |

---

## 6. 场景 4：知识页轻量增强（`WikiDocumentView.vue`）

三项增强均遵循“无异常不打扰、按需渐进披露”原则，不增加常驻侧栏或视觉噪音。

### 6.1 页头工具栏右侧轻量 L1 体检徽标

- **显隐规则**：仅当当前打开的知识页存在 **`severity === 'error' || severity === 'warn'`** 的未忽略体检项时显示；仅有 `info` 或全绿健康页时**完全不渲染**（零噪音）。
- **位置与视觉**：位于 `.doc-toolbar` 右侧操作按钮区中（`让我讲一遍` 与 `Obsidian` 之间，或紧挨模式切换左侧），采用紧凑可点击胶囊按钮：
  - 含 `error` 时：浅红底（`--error-soft`）+ `--error` 文字与 12px 线条图标，文案 `N 项体检异常`；
  - 仅 `warn` 时：浅琥珀底（`--warning-soft`）+ `--warning` 文字与 12px 线条图标，文案 `N 项体检提醒`。
- **交互**：点击后切至「整理与同步」页，自动滚动定位到「知识库体检」卡片并预置当前页过滤（`filterPagePath = doc.path`），用户可一键修复或切回全库。

### 6.2 同名双链点击消歧选择浮层

- **触发条件**：阅读模式下点击正文中的 `[[目标]]` 双链时，调用 `resolve_wikilink({ fromPath, target })`：
  - `unique`：直接打开目标页；
  - `missing`：轻量提示“未找到《目标》，可在整理或新建时创建”；
  - **`ambiguous`**：紧贴点击的链接下方弹出轻量消歧浮层（`.disambig-popover`，宽 `312px`，`max-width: calc(100vw - 24px)`；水平方向按阅读容器边界自动钳位，距左右边缘至少留 `12px`，避免靠近行首/行尾的链接弹出后越界裁切）。
- **浮层结构**：
  - 头部：`「{target}」对应 {N} 篇同名知识页` + 关闭按钮 `×`。
  - 候选列表：每项为单按钮行，展示 **页面标题** + **类型胶囊**（如 `决策` / `概念`）+ 第二行 **完整相对路径**（monospace，如 `wiki/decisions/超时设置.md`，启用 `word-break: break-all`）与最近更新日期。
  - 交互：单击任意项（或键盘 `↑`/`↓` + `Enter`）即打开该页并关闭浮层；点击空白处或按 `Esc` 关闭浮层。

### 6.3 悬停 `[^w1]` 脚注的轻量引文浮窗

- **触发条件**：鼠标悬停（延迟 `180ms` 防扫过误触）或键盘 `focus-visible` 聚焦正文内的 `[^w1]` 脚注徽标时，在徽标上方或下方弹出引文预览浮窗（`.citation-hover-card`，宽 `360px`，`max-width: calc(100vw - 24px)`；水平方向按阅读容器自动钳位距左右边缘至少 `12px`，垂直方向顶栏空间不足时自动下翻；提供 `8px` 透明悬停桥确保鼠标可平滑移入卡片点击按钮而不闪烁关闭）。
- **浮窗内容**（优先使用页面已加载的 `doc.citations` 秒出；按需可由 `read_citation_context` 补充核验状态）：
  - 顶部：来源原件标题 + 核验状态微胶囊（`已核对原件` `--success` / `原文有变动` `--warning`）+ 相对路径（`raw/notes/...`）。
  - 中部：左侧 accent 竖线包裹的**逐字短引**（`font-size: 12.5px`，最多展示 4 行，超出省略）。
  - 底部操作行：单行按钮 **`分屏对照并定位原段 →`**（点击脚注徽标本身或此按钮均打开 50/50 `.evidence-pane` 原文对照分屏，并自动滚动高亮该引用所在原文段落）。

---

## 7. 复用与增量 CSS Token 核查

所有增量样式严格派生自 [styles.css](styles.css) 既有变量（`--canvas`、`--inset`、`--surface`、`--surface-raised`、`--surface-active`、`--line-subtle`、`--line`、`--line-strong`、`--text`、`--text-secondary`、`--muted`、`--accent`、`--accent-soft`、`--accent-border`、`--accent-ink`、`--success/-soft`、`--warning/-soft`、`--error/-soft`、`--info/-soft`、`--shadow-float`、`--shadow-sm`），**不新增硬编码色值**，确保 Slate / Paper 双主题切换零失真。

| 验收维度 | 覆盖核查点 |
|---|---|
| 双主题对比度 | Cognitive Slate（深色默认）与 Paper Daylight（浅色）下全部 Diff 高亮、体检严重度胶囊、补全高亮行、代码框与浮窗文字均满足清晰可读对比度 |
| 1100×700 窄视口 | 主舞台 `1044px` 保持双栏时窄栏行均 `flex-wrap: wrap` + `word-break: break-all` 零横向溢出；`stage ≤ 860px` 折单栏、Diff `< 840px` 上下堆叠；设置弹窗滚动不溢出；`540×380`（`minHeight: 280`）随手记补全弹层 `fixed` 定位 + `min(220px, calc(100vh - 110px))` 边界钳位；知识页消歧与引文浮窗水平钳位不越出视口 |
| 键盘与无障碍 | 补全浮层 `role="listbox"` + `aria-selected`、消歧浮层与引文浮窗支持 `Esc` 关闭、全部图标为纯 inline SVG（`aria-hidden="true"`） |
| 文案洁净度 | 零 Emoji、零 `Changeset`/`EditOp`/`L1`/`L2`/`VERSION_CONFLICT` 英文工程词与报错命令直出给最终用户 |
