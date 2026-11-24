use std::io;

use tauri::WebviewWindow;

use crate::capture_commands::with_workspace;
use crate::dto::{AppError, CredentialStatusDto};
use crate::platform::{credential_exists, read_credential, write_credential, zero_string};
use crate::storage::Storage;

const SECRET_LIMIT: usize = 4096;
const SLOTS: [&str; 2] = ["model", "memory"];

#[tauri::command]
pub(crate) async fn list_credentials(
    window: WebviewWindow,
    session: String,
) -> Result<Vec<CredentialStatusDto>, AppError> {
    with_workspace(window, session, |storage| {
        SLOTS
            .into_iter()
            .map(|slot| status(storage, slot))
            .collect()
    })
    .await
}

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

#[tauri::command]
pub(crate) async fn save_credential(
    window: WebviewWindow,
    session: String,
    slot: String,
    mut secret: String,
) -> Result<CredentialStatusDto, AppError> {
    let slot = SLOTS
        .into_iter()
        .find(|name| *name == slot)
        .ok_or_else(|| AppError::invalid("credential", "凭据用途无效"))?;
    let mut trimmed = validate_secret(&secret)
        .map_err(|msg| AppError::invalid("credential", msg))?
        .to_owned();
    zero_string(&mut secret);
    with_workspace(window, session, move |storage| {
        let target = target_name(storage.workspace_id(), slot)?;
        let write_result = write_credential(&target, &trimmed);
        zero_string(&mut trimmed);
        write_result.map_err(|error| {
            AppError::new(
                "AUTH_FAILED",
                "credential",
                true,
                "无法写入 Windows 凭据管理器",
                Some(error.to_string()),
            )
        })?;
        storage.set_credential_reference(slot, Some(target.as_str()))?;
        Ok(CredentialStatusDto {
            slot: slot.to_owned(),
            target,
            state: "linked".to_owned(),
        })
    })
    .await
}

pub(crate) fn model_credential_state(storage: &Storage) -> Result<String, AppError> {
    Ok(status(storage, "model")?.state)
}

fn validate_secret(secret: &str) -> Result<&str, String> {
    let token = secret.trim();
    if token.is_empty()
        || token.len() > SECRET_LIMIT
        || !token.bytes().all(|b| b.is_ascii_graphic())
    {
        return Err("密钥不能为空，且仅允许可见 ASCII 字符（不能含内部空格或换行）".into());
    }
    Ok(token)
}

fn status(storage: &Storage, slot: &'static str) -> Result<CredentialStatusDto, AppError> {
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
    let target = target_name(storage.workspace_id(), "model")?;
    let linked = storage.credential_reference("model")?;
    if allow_anonymous && linked.is_none() {
        return Ok(None);
    }
    if linked.as_deref() != Some(target.as_str()) {
        return Err(AppError::from_message(
            "AUTH_FAILED",
            "credential",
            false,
            "请先在工作区设置关联模型凭据",
        ));
    }
    let token = read_credential(&target).map_err(credential_error)?;
    token.map(Some).ok_or_else(|| {
        AppError::from_message(
            "AUTH_FAILED",
            "credential",
            false,
            "模型凭据为空或格式无效，请在工作区设置或 Windows 凭据管理器中更新",
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
        assert_eq!(validate_secret("  sk-test-123\r\n").unwrap(), "sk-test-123");
        assert!(validate_secret("   ").is_err());
        assert!(validate_secret("sk test").is_err());
        assert!(validate_secret("sk-\u{0007}").is_err());
    }
}
