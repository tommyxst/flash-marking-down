# ADR-02 模型与 OpenViking 记忆接入决策

更新于 2026-11-24（v1.0 重构修订）。

## 1. HTTP 传输栈统一复用 `reqwest`

- `Cargo.lock` 已由 Tauri 依赖链锁定 `reqwest 0.13.5`。自 v1.0（R1）起在 `src-tauri/Cargo.toml` 中显式声明 `reqwest`（保留 `platform::HttpTransport` 抽象以便纯内存 Mock 单测），替代手写的 ~160 行 `unsafe` WinHTTP FFI。
- 收益：显著降低 Windows 专属 `unsafe` 维护风险，原生支持 LLM 非流式 JSON 整合请求、对话流式输出（SSE）以及 OpenViking HTTP API 调用，并为未来 macOS 扩展消除传输层差异。

## 2. 大模型接入（OpenAI 兼容 Chat Completions）

- **端点与凭据**：工作区保存完整 Chat Completions URL（允许常规 query 参数如 Azure `api-version`，禁止 URL 内嵌凭据或片段）、模型名与启用状态。公网端点要求 HTTPS 并配置 API Key（经 Windows `CredWriteW` 写入系统凭据管理器，输入校验同步兼顾 UTF-16 编码字节上限）；本机回环（`localhost` / `127.0.0.1` / `[::1]`）与局域网私有地址允许 HTTP 及按需免凭据。
- **体验约定**：
  - 设置弹窗内将模型配置与 API Key 合并为一次提交，并提供一键「测试连接」。
  - 启用后，后台自动整理与右侧多轮对话直接发起请求，不弹逐次确认窗。
  - 结构化整理请求保持非流式 JSON（含容错提取与最多 1 次格式重试）；右侧对话面板支持流式输出（SSE）以降低首字等待感。
  - 模型仅接收授权文本上下文，不赋予工具调用或自主文件读写权限。

## 3. 记忆连接器抽象与 OpenViking v0.5.2 双路整合

- **接口解耦**：在 `src-tauri/src/memory/` 定义 `MemoryProvider` trait，隔离核心业务逻辑与具体记忆服务。一期全力打透 `OpenVikingProvider`，暂不为第二种真实记忆服务投入额外验收开发。
- **本机 OpenViking v0.5.2 API 对齐**：
  - **认证**：支持 `X-API-Key`（默认）或 `Authorization: Bearer <token>`，API Key 同样存 Windows 系统凭据管理器（槽位 `memory`）。
  - **连通性测试**：`GET /health`（携带认证头解析 `account_id / user_id / role`），支撑设置页一键测试连接。
  - **知识原文路**：笔记落盘每日文件后，后台异步调用 `POST /api/v1/content/write`（`mode: "replace"`, `wait: false`），写入工作区隔离前缀 `viking://resources/flash-marking-down/<workspace>/daily/<YYYY-MM-DD>.md`，利用 OV 自动生成 L0/L1 分层摘要与向量索引。
  - **长期记忆路**：后台整理完成后，将本次新笔记要点通过 `POST /api/v1/sessions` → `POST /api/v1/sessions/{id}/messages` → `POST /api/v1/sessions/{id}/commit`（`wait: false`）交由 OV 记忆管线抽取并去重沉淀为长期 `memories`。
  - **混合对话召回**：对话提问时并行检索本地 FTS5 与 `POST /api/v1/search/search`（召回 `memories` 与 `resources`），按需通过 `GET /api/v1/content/overview` 或 `GET /api/v1/content/read` 展开细节。
- **务实容错**：不引入重型异步双验证 Saga 对账矩阵。OV 离线、超时或鉴权失败时，本地记录轻量 `pending | failed` 同步状态供自动重试或手动一键补推；对话自动降级为纯本地笔记召回并温和提示。
