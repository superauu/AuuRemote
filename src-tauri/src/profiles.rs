use std::path::Path;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::{
    errors::{ApiError, ErrorCode},
    rdp::Endpoint,
    store::{config_directory, read_json_file, with_file_lock, write_json_file},
};

const DATA_NAME: &str = "connections.json";
const LOCK_NAME: &str = "connections.lock";

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Details {
    name: String,
    #[serde(flatten)]
    endpoint: Endpoint,
    /// "auto" (fit the window) or "WIDTHxHEIGHT"; defaults keep old files valid.
    #[serde(default = "default_resolution")]
    resolution: String,
}

fn default_resolution() -> String {
    "auto".to_owned()
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Profile {
    id: u32,
    #[serde(flatten)]
    details: Details,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Profiles {
    last_id: u32,
    entries: Vec<Profile>,
}

impl Profiles {
    fn save(&mut self, id: Option<u32>, mut details: Details) -> Result<u32, ApiError> {
        details.name = details.name.trim().to_owned();
        if details.name.is_empty() || details.name.chars().count() > 80 {
            return Err(ApiError::new(
                ErrorCode::InvalidProfileName,
                "连接名称须为 1 至 80 个字符。",
            ));
        }
        details.endpoint.validate()?;

        if let Some(id) = id {
            let profile = self
                .entries
                .iter_mut()
                .find(|profile| profile.id == id)
                .ok_or_else(|| {
                    ApiError::new(ErrorCode::ProfileMissing, "这条连接已被删除。")
                })?;
            profile.details = details;
            return Ok(id);
        }

        let id = self.last_id.checked_add(1).ok_or_else(|| {
            ApiError::new(ErrorCode::Internal, "连接编号已用完。")
        })?;
        self.entries.push(Profile { id, details });
        self.last_id = id;
        Ok(id)
    }
}

fn update_store<T>(
    directory: &Path,
    operation: impl FnOnce(&mut Profiles) -> Result<(T, bool), ApiError>,
) -> Result<T, ApiError> {
    with_file_lock(directory, LOCK_NAME, || {
        let mut profiles = read_json_file::<Profiles>(&directory.join(DATA_NAME))?.unwrap_or_default();

        let (result, changed) = operation(&mut profiles)?;
        if changed {
            write_json_file(&directory.join(DATA_NAME), &profiles)?;
        }
        Ok(result)
    })
}

async fn with_store<T: Send + 'static>(
    app: AppHandle,
    operation: impl FnOnce(&mut Profiles) -> Result<(T, bool), ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    let directory = config_directory(&app)?;
    tauri::async_runtime::spawn_blocking(move || update_store(&directory, operation))
        .await
        .map_err(|error| ApiError::chain(ErrorCode::Internal, &error, "连接配置任务失败"))?
}

#[tauri::command]
pub(crate) async fn list_profiles(app: AppHandle) -> Result<Vec<Profile>, ApiError> {
    with_store(app, |store| Ok((store.entries.clone(), false))).await
}

#[derive(Serialize)]
pub(crate) struct Saved {
    id: u32,
    profiles: Vec<Profile>,
}

#[tauri::command]
pub(crate) async fn save_profile(
    app: AppHandle,
    id: Option<u32>,
    details: Details,
) -> Result<Saved, ApiError> {
    with_store(app, move |store| {
        let id = store.save(id, details)?;
        // Return the same snapshot that was persisted.
        Ok((
            Saved {
                id,
                profiles: store.entries.clone(),
            },
            true,
        ))
    })
    .await
}

#[tauri::command]
pub(crate) async fn remove_profile(app: AppHandle, id: u32) -> Result<Vec<Profile>, ApiError> {
    with_store(app, move |store| {
        let before = store.entries.len();
        store.entries.retain(|profile| profile.id != id);
        let changed = store.entries.len() != before;
        Ok((store.entries.clone(), changed))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Details {
        Details {
            name: name.into(),
            endpoint: Endpoint {
                host: " [::1] ".into(),
                port: 3389,
                username: " fixture ".into(),
                fingerprint: "ab".repeat(32),
            },
            resolution: "auto".into(),
        }
    }

    #[test]
    fn edits_persist_and_stale_ids_cannot_recreate_a_removed_profile() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let id = update_store(dir.path(), |store| {
            Ok((store.save(None, fixture("Desk"))?, true))
        })?;
        update_store(dir.path(), |store| {
            Ok((store.save(Some(id), fixture("Lab"))?, true))
        })?;
        let entries = update_store(dir.path(), |store| Ok((store.entries.clone(), false)))?;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].details.name, "Lab");
        assert_eq!(entries[0].details.endpoint.host, "::1");
        assert_eq!(entries[0].details.endpoint.username, "fixture");
        assert!(entries[0].details.endpoint.fingerprint.starts_with("AB:AB"));

        update_store(dir.path(), |store| {
            store.entries.retain(|entry| entry.id != id);
            Ok(((), true))
        })?;
        let result = update_store(dir.path(), |store| {
            Ok((store.save(Some(id), fixture("Outdated"))?, true))
        });
        assert!(result.is_err());
        let next = update_store(dir.path(), |store| {
            Ok((store.save(None, fixture("New"))?, true))
        })?;
        assert_ne!(next, id);
        Ok(())
    }

    #[test]
    fn imported_extra_fields_are_excluded_and_corrupt_storage_is_not_overwritten()
    -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let details: Details = serde_json::from_str(
            r#"{
            "name": "Desk", "host": "localhost", "port": 3389, "username": "fixture",
            "password": "fixture-only"
        }"#,
        )?;
        update_store(dir.path(), |store| Ok((store.save(None, details)?, true)))?;
        let path = dir.path().join(DATA_NAME);
        let json = std::fs::read_to_string(&path)?;
        assert!(!json.contains("password") && !json.contains("fixture-only"));
        std::fs::write(&path, b"{ damaged")?;
        let result = update_store(dir.path(), |store| {
            Ok((store.save(None, fixture("New"))?, true))
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(&path)?, "{ damaged");
        Ok(())
    }

    #[test]
    fn concurrent_app_instances_keep_both_saves() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let directory = dir.path();
        std::thread::scope(|threads| {
            let save = |name| {
                update_store(directory, |store| {
                    Ok((store.save(None, fixture(name))?, true))
                })
            };
            let first = threads.spawn(move || save("One"));
            let second = threads.spawn(move || save("Two"));
            assert_ne!(first.join().unwrap()?, second.join().unwrap()?);
            anyhow::Ok(())
        })?;
        let count = update_store(directory, |store| Ok((store.entries.len(), false)))?;
        assert_eq!(count, 2);
        Ok(())
    }
}
