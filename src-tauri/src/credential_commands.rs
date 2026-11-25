use std::io;

use tauri::WebviewWindow;

use crate::capture_commands::with_workspace;
use crate::dto::{AppError, CredentialStatusDto};
use crate::platform::{bearer_token, credential_exists, read_credential};
use crate::storage::Storage;

const CRED_MAX_CREDENTIAL_BLOB_SIZE: usize = 5 * 512;
const SLOTS: [&str; 2] = ["model", "memory"];

#[tauri::command]
pub(crate) async fn set_credential(
    window: WebviewWindow,
    session: String,
    slot: String,
    linked: bool,
) -> Result<CredentialStatusDto, AppError> {
    let slot = SLOTS
        .into_iter()
        .find(|name| *name == slot)
        .ok_or_else(|| AppError::invalid("credential", "凭据用途无效"))?;
    with_workspace(window, session, move |storage| {
        let target = target_name(storage.workspace_id(), slot)?;
        if linked && !credential_exists(&target).map_err(credential_error)? {
            return Err(AppError::from_message(
                "AUTH_FAILED",
                "credential",
                false,
                "Windows 凭据管理器中找不到此普通凭据",
            ));
        }
        storage.set_credential_reference(slot, linked.then_some(target.as_str()))?;
        Ok(CredentialStatusDto {
            slot: slot.to_owned(),
            target,
            state: if linked { "linked" } else { "unlinked" }.to_owned(),
        })
    })
    .await
}

pub(crate) fn config_key<'a>(
    storage: &Storage,
    slot: &str,
    secret: &'a str,
) -> Result<Option<(String, &'a str)>, AppError> {
    if secret.trim().is_empty() {
        return Ok(None);
    }
    let token = validate_secret(secret, slot == "memory")
        .map_err(|msg| AppError::invalid("credential", msg))?;
    Ok(Some((target_name(storage.workspace_id(), slot)?, token)))
}

pub(crate) fn model_credential_state(storage: &Storage) -> Result<String, AppError> {
    Ok(status(storage, "model")?.state)
}

fn validate_secret(secret: &str, allow_bearer: bool) -> Result<&str, String> {
    let token = secret.trim();
    let key = if allow_bearer {
        bearer_token(token).unwrap_or(token)
    } else {
        token
    };
    if key.is_empty() || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("密钥不能为空，且仅允许可见 ASCII 字符（不能含内部空格或换行）".into());
    }
    if token.encode_utf16().count() * size_of::<u16>() > CRED_MAX_CREDENTIAL_BLOB_SIZE {
        return Err("密钥超过 Windows 凭据管理器的 UTF-16 上限（2560 字节）".into());
    }
    Ok(token)
}

pub(crate) fn status(
    storage: &Storage,
    slot: &'static str,
) -> Result<CredentialStatusDto, AppError> {
    let target = target_name(storage.workspace_id(), slot)?;
    let reference = storage.credential_reference(slot)?;
    let state = match reference.as_deref() {
        None => "unlinked",
        Some(reference) if reference == target => match credential_exists(&target) {
            Ok(true) => "linked",
            Ok(false) => "missing",
            Err(_) => "unavailable",
        },
        Some(_) => "invalid",
    };
    Ok(CredentialStatusDto {
        slot: slot.to_owned(),
        target,
        state: state.to_owned(),
    })
}

fn target_name(workspace_id: &str, slot: &str) -> Result<String, AppError> {
    if workspace_id.len() != 32 || !workspace_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AppError::invalid(
            "credential",
            "工作区标识无效，无法关联凭据",
        ));
    }
    Ok(format!("FlashMarkingDown/{workspace_id}/{slot}"))
}

fn credential_error(error: io::Error) -> AppError {
    AppError::new(
        "AUTH_FAILED",
        "credential",
        true,
        "无法查询 Windows 凭据管理器",
        Some(error.to_string()),
    )
}

pub(crate) fn model_token(
    storage: &Storage,
    allow_anonymous: bool,
) -> Result<Option<String>, AppError> {
    service_token(storage, "model", allow_anonymous)
}

pub(crate) fn service_token(
    storage: &Storage,
    slot: &str,
    allow_anonymous: bool,
) -> Result<Option<String>, AppError> {
    let target = target_name(storage.workspace_id(), slot)?;
    let linked = storage.credential_reference(slot)?;
    if allow_anonymous && linked.is_none() {
        return Ok(None);
    }
    if linked.as_deref() != Some(target.as_str()) {
        return Err(AppError::from_message(
            "AUTH_FAILED",
            "credential",
            false,
            "请先在工作区设置录入或关联此服务的 API Key",
        ));
    }
    let token = read_credential(&target, slot == "memory").map_err(credential_error)?;
    token.map(Some).ok_or_else(|| {
        AppError::from_message(
            "AUTH_FAILED",
            "credential",
            false,
            "凭据为空或格式无效，请在工作区设置或 Windows 凭据管理器中更新",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_target_is_scoped_and_missing_is_distinct() {
        let now = std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos();
        let id = format!("{:08x}{now:024x}", std::process::id());
        let model = target_name(&id, "model").unwrap();
        let memory = target_name(&id, "memory").unwrap();
        assert_eq!(model, format!("FlashMarkingDown/{id}/model"));
        assert_eq!(memory, format!("FlashMarkingDown/{id}/memory"));
        assert!(target_name("../outside", "model").is_err());
        assert!(!credential_exists(&model).unwrap());
    }

    #[test]
    fn validates_and_trims_credential_secrets() {
        assert!(validate_secret(&"x".repeat(1280), false).is_ok());
        assert!(
            validate_secret(&"x".repeat(1281), false)
                .unwrap_err()
                .contains("UTF-16")
        );
        assert_eq!(
            validate_secret("  sk-test-123\r\n", false).unwrap(),
            "sk-test-123"
        );
        assert!(validate_secret("   ", false).is_err());
        assert!(validate_secret("sk test", false).is_err());
        assert!(validate_secret("sk-\u{0007}", false).is_err());
        assert_eq!(
            validate_secret("  bEaReR fixture  ", true).unwrap(),
            "bEaReR fixture"
        );
        assert!(validate_secret("Bearer fixture other", true).is_err());
        assert!(validate_secret("Bearer fixture", false).is_err());
    }
}
