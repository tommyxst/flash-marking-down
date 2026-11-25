use serde::{Deserialize, Serialize};

use crate::storage::StorageError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub stage: String,
    pub operation_id: Option<String>,
    pub retryable: bool,
    pub safe_message: String,
    pub technical_detail: Option<String>,
}

impl AppError {
    pub fn new(
        code: impl Into<String>,
        stage: impl Into<String>,
        retryable: bool,
        safe_message: impl Into<String>,
        technical_detail: Option<String>,
    ) -> Self {
        Self {
            code: code.into(),
            stage: stage.into(),
            operation_id: None,
            retryable,
            safe_message: safe_message.into(),
            technical_detail: technical_detail.filter(|detail| !detail.trim().is_empty()),
        }
    }

    pub fn invalid(stage: &'static str, safe_message: impl Into<String>) -> Self {
        Self::new("INVALID_INPUT", stage, false, safe_message, None)
    }

    pub fn from_message(
        code: &'static str,
        stage: &'static str,
        retryable: bool,
        safe_message: impl Into<String>,
    ) -> Self {
        Self::new(code, stage, retryable, safe_message, None)
    }
}

impl From<StorageError> for AppError {
    fn from(error: StorageError) -> Self {
        let detail =
            if error.technical_detail.is_empty() || error.technical_detail == error.safe_message {
                None
            } else {
                Some(error.technical_detail)
            };
        Self {
            code: error.code.to_owned(),
            stage: error.stage.to_owned(),
            operation_id: error.operation_id,
            retryable: error.retryable,
            safe_message: error.safe_message.to_owned(),
            technical_detail: detail,
        }
    }
}

impl From<String> for AppError {
    fn from(message: String) -> Self {
        Self::new("OPERATION_FAILED", "desktop", true, message, None)
    }
}

impl From<&str> for AppError {
    fn from(message: &str) -> Self {
        Self::new("OPERATION_FAILED", "desktop", true, message, None)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DraftDto {
    pub revision: i64,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryErrorDto {
    pub request_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpenedWorkspaceDto {
    pub session: String,
    pub root: String,
    pub capture_dir: String,
    pub draft: DraftDto,
    pub recovered: Vec<String>,
    pub recovery_errors: Vec<RecoveryErrorDto>,
    pub pending_request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecentCaptureDto {
    pub path: String,
    pub block_id: Option<String>,
    pub captured_at: String,
    pub body: String,
    pub content_hash: String,
    pub is_capture: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchHitDto {
    pub path: String,
    pub captured_at: Option<String>,
    pub content_hash: String,
    pub preview: String,
    pub title_text: String,
    pub is_capture: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchPageDto {
    pub hits: Vec<SearchHitDto>,
    pub has_more: bool,
    pub skipped: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IndexRebuildDto {
    pub indexed: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecordSourceDto {
    pub state: String,
    pub content_hash: String,
    pub body: String,
    pub is_capture: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyChoiceDto {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyStatusDto {
    pub id: String,
    pub active: bool,
    pub error: Option<String>,
    pub choices: Vec<HotkeyChoiceDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CredentialStatusDto {
    pub slot: String,
    pub target: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfigDto {
    pub endpoint: String,
    pub model: String,
    pub enabled: bool,
    pub ready: bool,
    pub requires_credential: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WikiTargetDto {
    pub kind: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WikiChangeDto {
    pub id: String,
    pub target: String,
    pub kind: String,
    pub phase: String,
    pub base_hash: String,
    pub before: String,
    pub after: String,
    pub source_path: String,
    pub source_revision: String,
    pub start_line: u32,
    pub end_line: u32,
    pub source_excerpt: String,
    pub source_is_capture: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WikiApplyOutcomeDto {
    pub id: String,
    pub outcome: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskEntryDto {
    pub id: String,
    pub expected_line: String,
    pub text: String,
    pub state: String,
    pub source_path: String,
    pub source_is_capture: bool,
    pub manual: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskListDto {
    pub path: String,
    pub revision: String,
    pub entries: Vec<TaskEntryDto>,
    pub repair_messages: Vec<String>,
}
