use serde::Serialize;

use super::ConflictDto;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LibraryDocSummaryDto {
    pub path: String,
    pub title: String,
    pub folder: String,
    pub doc_type: String,
    pub type_label: &'static str,
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
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChangesetOpDto {
    pub label: String,
    pub target_path: String,
    pub target_title: String,
    pub outcome: &'static str,
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
    pub page_paths: Vec<String>,
    pub ops: Vec<ChangesetOpDto>,
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
