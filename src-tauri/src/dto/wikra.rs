use serde::{Deserialize, Serialize};

use super::ConflictDto;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LibraryDocSummaryDto {
    pub path: String,
    pub title: String,
    pub folder: String,
    pub doc_type: String,
    pub type_label: &'static str,
    pub aliases: Vec<String>,
    pub status: Option<String>,
    pub tags: Vec<String>,
    pub updated_at: Option<String>,
    pub citation_count: usize,
    pub pending_conflict_count: usize,
    pub wikra_core: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawFileSummaryDto {
    pub id: String,
    pub path: String,
    pub title: String,
    pub kind: &'static str,
    pub kind_label: &'static str,
    pub created_at: Option<String>,
    pub source_url: Option<String>,
    pub linked_page_paths: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LibraryTreeDto {
    pub raw_dir: String,
    pub wiki_pages: Vec<LibraryDocSummaryDto>,
    pub raw_files: Vec<RawFileSummaryDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DocCitationDto {
    pub id: String,
    pub ref_label: String,
    pub raw_path: String,
    pub label: String,
    pub quote: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DocBacklinkDto {
    pub page_path: String,
    pub title: String,
    pub excerpt: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DocTaskItemDto {
    pub id: String,
    pub line_number: usize,
    pub expected_line: String,
    pub text: String,
    pub done: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DocumentDetailDto {
    pub kind: &'static str,
    pub path: String,
    pub title: String,
    pub body: String,
    pub content_hash: String,
    pub updated_at: Option<String>,
    pub doc_type: String,
    pub type_label: &'static str,
    pub aliases: Vec<String>,
    pub status: Option<String>,
    pub supersedes: Vec<String>,
    pub tags: Vec<String>,
    pub wikra_core: bool,
    pub source_url: Option<String>,
    pub pending_conflict_ids: Vec<String>,
    pub outline: Vec<String>,
    pub tasks: Vec<DocTaskItemDto>,
    pub citations: Vec<DocCitationDto>,
    pub backlinks: Vec<DocBacklinkDto>,
    pub linked_wiki_paths: Vec<String>,
    pub has_protected_section: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InboxParagraphDto {
    pub id: String,
    pub heading: String,
    pub text: String,
    pub highlighted: bool,
    pub compiled: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InboxItemSummaryDto {
    pub id: String,
    pub title: String,
    pub domain: String,
    pub source_type: String,
    pub url: Option<String>,
    pub raw_path: String,
    pub saved_at: String,
    pub read_time: String,
    pub fetch_status: String,
    pub state: String,
    pub word_count: u32,
    pub highlight_count: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InboxItemDetailDto {
    #[serde(flatten)]
    pub summary: InboxItemSummaryDto,
    pub summary_points: Vec<String>,
    pub related_page_paths: Vec<String>,
    pub paragraphs: Vec<InboxParagraphDto>,
    pub compile_summary: Option<CompileSummaryDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompileSummaryDto {
    pub changeset_id: String,
    pub applied: usize,
    pub pending: usize,
    pub dropped: usize,
    pub remaining_paragraphs: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChangesetOpDto {
    pub seq: i64,
    pub op: String,
    pub label: String,
    pub target_path: String,
    pub target_title: String,
    pub outcome: String,
    pub reason: String,
    pub conflict_id: Option<String>,
    pub child_changeset_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChangesetItemDto {
    pub id: String,
    pub time: String,
    pub trigger: String,
    pub model_alias: String,
    pub status: String,
    pub summary: String,
    pub note: String,
    pub parent_id: Option<String>,
    pub dropped_claims: usize,
    pub page_paths: Vec<String>,
    pub pages: Vec<ChangesetPageDto>,
    pub ops: Vec<ChangesetOpDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChangesetPageDto {
    pub path: String,
    pub title: String,
    pub state: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChangesetPageDiffDto {
    #[serde(flatten)]
    pub page: ChangesetPageDto,
    pub has_pending: bool,
    pub pending_reasons: Vec<String>,
    pub has_protected_section: bool,
    pub before: Option<String>,
    pub after: Option<String>,
    pub current: Option<String>,
    pub externally_modified: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChangesetDetailDto {
    pub changeset: ChangesetItemDto,
    pub pages: Vec<ChangesetPageDiffDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReviewDecision {
    Apply,
    Discard,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RevertChangesetResultDto {
    pub changeset: ChangesetItemDto,
    pub restored: Vec<String>,
    pub kept_modified: Vec<String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SemanticLintScope {
    Page { path: String },
    Recent { days: Option<u32> },
    Folder { path: String },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LintFindingFixDto {
    pub kind: String,
    pub label: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LintEvidenceDto {
    pub quote_a: String,
    pub quote_b: String,
    pub path_a: String,
    pub path_b: String,
    pub title_a: String,
    pub title_b: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LintFindingDto {
    pub id: String,
    pub layer: String,
    pub rule: String,
    pub severity: String,
    pub page_path: String,
    pub page_title: String,
    pub line: Option<usize>,
    pub related_paths: Vec<String>,
    pub message: String,
    pub dismissed: bool,
    pub fix: Option<LintFindingFixDto>,
    pub evidence: Option<LintEvidenceDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LintReportDto {
    pub generated_at: String,
    pub page_count: usize,
    pub unreadable: usize,
    pub duration_ms: u64,
    pub findings: Vec<LintFindingDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LintFixResultDto {
    pub changeset: Option<ChangesetItemDto>,
    pub index_rebuilt: bool,
    pub report: LintReportDto,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SemanticLintPreviewDto {
    pub pages: Vec<String>,
    pub page_count: usize,
    pub pair_count: usize,
    pub estimated_chars: usize,
    pub model_alias: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WikilinkCandidateDto {
    pub path: String,
    pub stem: String,
    pub title: String,
    pub aliases: Vec<String>,
    pub folder: String,
    pub type_label: &'static str,
    pub link_text: String,
    pub ambiguous_stem: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WikilinkMatchDto {
    pub path: String,
    pub title: String,
    pub type_label: &'static str,
    pub updated_at: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WikilinkResolutionDto {
    pub status: &'static str,
    pub matches: Vec<WikilinkMatchDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CitationContextDto {
    pub raw_path: String,
    pub quote: String,
    pub verified: bool,
    pub paragraph: Option<String>,
    pub prev: Option<String>,
    pub next: Option<String>,
    pub reason: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpSnippetsDto {
    pub claude_cli: String,
    pub mcp_json: String,
    pub codex_toml: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawDirOptionDto {
    pub path: String,
    pub file_count: usize,
    pub is_capture_dir: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentIntegrationDto {
    pub mcp_exe_path: String,
    pub mcp_exe_exists: bool,
    pub vault_root: String,
    pub raw_dir: String,
    pub raw_dirs: Vec<RawDirOptionDto>,
    pub raw_allow: Vec<String>,
    pub snippets: McpSnippetsDto,
    pub skill_path: String,
    pub skill_exists: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportSkillResultDto {
    pub path: String,
    pub action: &'static str,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpTestResultDto {
    pub ok: bool,
    pub protocol_version: Option<String>,
    pub tools: Vec<String>,
    pub error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BufferQueueItemDto {
    pub id: String,
    pub note_id: String,
    pub path: String,
    pub title: String,
    pub preview: String,
    pub queued_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrganizeSuggestionDto {
    pub id: String,
    pub kind: &'static str,
    pub kind_label: &'static str,
    pub title: String,
    pub detail: String,
    pub action_label: &'static str,
    pub page_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrganizeOverviewDto {
    pub conflicts: Vec<ConflictDto>,
    pub changesets: Vec<ChangesetItemDto>,
    pub suggestions: Vec<OrganizeSuggestionDto>,
    pub buffer_queue: Vec<BufferQueueItemDto>,
    pub buffer_seconds_left: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureCommitResultDto {
    pub path: String,
    pub note_id: String,
    pub queued: bool,
    pub buffer_seconds_left: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TeachbackTurnDto {
    pub step: u8,
    pub total_steps: u8,
    pub prompt: String,
    pub feedback: String,
    pub suggested_insight: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphNodeDto {
    pub id: String,
    pub title: String,
    pub doc_type: String,
    pub degree: usize,
    pub is_hub: bool,
    pub is_orphan: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphEdgeDto {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: &'static str,
    pub label: String,
    pub snippet: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphGapDto {
    pub id: String,
    pub title: String,
    pub reason: String,
    pub question: String,
    pub from_path: String,
    pub to_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KnowledgeGraphDto {
    pub nodes: Vec<GraphNodeDto>,
    pub edges: Vec<GraphEdgeDto>,
    pub gaps: Vec<GraphGapDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GlobalSearchHitDto {
    pub kind: &'static str,
    pub kind_label: &'static str,
    pub title: String,
    pub subtitle: String,
    pub target_path: String,
    pub citation_id: Option<String>,
}
