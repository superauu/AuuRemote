use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufReader},
    path::Path,
};

use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::rdp::Endpoint;

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Details {
    name: String,
    #[serde(flatten)]
    endpoint: Endpoint,
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
    fn save(&mut self, id: Option<u32>, mut details: Details) -> anyhow::Result<u32> {
        details.name = details.name.trim().to_owned();
        ensure!(
            !details.name.is_empty() && details.name.chars().count() <= 80,
            "连接名称须为 1 至 80 个字符。"
        );
        details.endpoint.validate().map_err(anyhow::Error::msg)?;

        if let Some(id) = id {
            let profile = self
                .entries
                .iter_mut()
                .find(|profile| profile.id == id)
                .context("这条连接已被删除。")?;
            profile.details = details;
            return Ok(id);
        }

        let id = self.last_id.checked_add(1).context("连接编号已用完。")?;
        self.entries.push(Profile { id, details });
        self.last_id = id;
        Ok(id)
    }
}

fn update_store<T>(
    directory: &Path,
    operation: impl FnOnce(&mut Profiles) -> anyhow::Result<(T, bool)>,
) -> anyhow::Result<T> {
    fs::create_dir_all(directory).context("无法创建连接配置目录")?;

    // Serialize reads and modifications across app instances. The lock is
    // separate from the data file, which is replaced after each write.
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("connections.lock"))
        .context("无法打开连接配置锁")?;
    lock.lock().context("无法锁定连接配置")?;

    let path = directory.join("connections.json");
    let mut profiles = match File::open(&path) {
        Ok(file) => serde_json::from_reader(BufReader::new(file)).context("连接配置无法读取")?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Profiles::default(),
        Err(error) => return Err(error).context("无法打开连接配置"),
    };

    let (result, changed) = operation(&mut profiles)?;
    if changed {
        let mut temp =
            tempfile::NamedTempFile::new_in(directory).context("无法创建连接配置临时文件")?;
        serde_json::to_writer_pretty(temp.as_file_mut(), &profiles).context("无法写入连接配置")?;
        temp.as_file().sync_all().context("无法同步连接配置")?;
        temp.persist(path).context("无法替换连接配置")?;
    }
    Ok(result)
}

async fn with_store<T: Send + 'static>(
    app: AppHandle,
    operation: impl FnOnce(&mut Profiles) -> anyhow::Result<(T, bool)> + Send + 'static,
) -> Result<T, String> {
    let directory = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("无法找到应用配置目录。{error}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        update_store(&directory, operation).map_err(|error| format!("{error:#}"))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn list_profiles(app: AppHandle) -> Result<Vec<Profile>, String> {
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
) -> Result<Saved, String> {
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
pub(crate) async fn remove_profile(app: AppHandle, id: u32) -> Result<Vec<Profile>, String> {
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
        let path = dir.path().join("connections.json");
        let json = fs::read_to_string(&path)?;
        assert!(!json.contains("password") && !json.contains("fixture-only"));
        fs::write(&path, b"{ damaged")?;
        let result = update_store(dir.path(), |store| {
            Ok((store.save(None, fixture("New"))?, true))
        });
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path)?, "{ damaged");
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
