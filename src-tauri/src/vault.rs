use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, generic_array::GenericArray},
};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

use crate::{
    errors::{ApiError, ErrorCode},
    store::{config_directory, read_json_file, with_file_lock, write_json_file},
};

const DATA_NAME: &str = "secrets.json";
const LOCK_NAME: &str = "secrets.lock";
// Fixed plaintext used as the master-password verifier: decrypting it proves
// the entered password re-derived the right key.
const VERIFIER_PLAINTEXT: &[u8] = b"AuuRemote master-password vault v1";
const MASTER_PASSWORD_MIN_CHARS: usize = 8;
// Argon2id parameters from the OWASP Password Storage Cheat Sheet.
const KDF_M_COST_KIB: u32 = 19456;
const KDF_T_COST: u32 = 2;
const KDF_P_COST: u32 = 1;
const KEY_LEN: usize = 32;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;

type KeyCell = Arc<Mutex<Option<Zeroizing<[u8; KEY_LEN]>>>>;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VaultFile {
    kdf: Kdf,
    verifier: String,
    entries: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Kdf {
    salt: String,
    m_cost_kib: u32,
    t_cost: u32,
    p_cost: u32,
}

/// Unlocked vault keys live only in memory; they are zeroized when replaced,
/// on lock, and on app exit. The `Arc` lets commands move a handle into
/// blocking tasks.
pub(crate) struct Vault {
    key: KeyCell,
}

impl Default for Vault {
    fn default() -> Self {
        Self {
            key: Arc::new(Mutex::new(None)),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VaultStatus {
    setup: bool,
    unlocked: bool,
    profile_ids: Vec<u32>,
}

fn status_of(directory: &Path, key: &KeyCell) -> Result<VaultStatus, ApiError> {
    let file = load_vault(directory)?;
    let unlocked = key.lock().expect("vault lock").is_some();
    Ok(VaultStatus {
        setup: file.is_some(),
        unlocked,
        profile_ids: file
            .map(|file| {
                file.entries
                    .keys()
                    .filter_map(|key| key.parse().ok())
                    .collect()
            })
            .unwrap_or_default(),
    })
}

fn load_vault(directory: &Path) -> Result<Option<VaultFile>, ApiError> {
    read_json_file::<VaultFile>(&directory.join(DATA_NAME))
}

fn write_vault(directory: &Path, file: &VaultFile) -> Result<(), ApiError> {
    with_file_lock(directory, LOCK_NAME, || {
        write_json_file(&directory.join(DATA_NAME), file)
    })
}

fn random_bytes(len: usize) -> Result<Vec<u8>, ApiError> {
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes)
        .map_err(|error| ApiError::chain(ErrorCode::Internal, &error, "无法生成随机数"))?;
    Ok(bytes)
}

fn fresh_kdf() -> Result<Kdf, ApiError> {
    Ok(Kdf {
        salt: BASE64.encode(random_bytes(SALT_LEN)?),
        m_cost_kib: KDF_M_COST_KIB,
        t_cost: KDF_T_COST,
        p_cost: KDF_P_COST,
    })
}

fn derive_key(password: &str, kdf: &Kdf) -> Result<Zeroizing<[u8; KEY_LEN]>, ApiError> {
    let salt = BASE64
        .decode(&kdf.salt)
        .map_err(|error| ApiError::chain(ErrorCode::ConfigCorrupt, &error, "保险库 KDF 盐无效"))?;
    let params = Params::new(kdf.m_cost_kib, kdf.t_cost, kdf.p_cost, Some(KEY_LEN))
        .map_err(|error| ApiError::chain(ErrorCode::ConfigCorrupt, &error, "保险库 KDF 参数无效"))?;
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), &salt, key.as_mut())
        .map_err(|error| ApiError::chain(ErrorCode::Internal, &error, "无法派生保险库密钥"))?;
    Ok(key)
}

fn encrypt_b64(key: &[u8; KEY_LEN], plaintext: &[u8]) -> Result<String, ApiError> {
    let cipher = Aes256Gcm::new(GenericArray::from_slice(key));
    let nonce = random_bytes(NONCE_LEN)?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|_| ApiError::new(ErrorCode::Internal, "无法加密保险库数据"))?;
    let mut blob = nonce;
    blob.extend(ciphertext);
    Ok(BASE64.encode(blob))
}

fn decrypt_b64(key: &[u8; KEY_LEN], blob: &str) -> Result<Vec<u8>, ApiError> {
    let data = BASE64
        .decode(blob)
        .map_err(|_| ApiError::new(ErrorCode::VaultWrongPassword, "保险库密文无效。"))?;
    let Some((nonce, ciphertext)) = data.split_at_checked(NONCE_LEN) else {
        return Err(ApiError::new(ErrorCode::VaultWrongPassword, "保险库密文无效。"));
    };
    let cipher = Aes256Gcm::new(GenericArray::from_slice(key));
    cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| ApiError::new(ErrorCode::VaultWrongPassword, "主密码不正确。"))
}

fn unlocked_key(cell: &KeyCell) -> Result<Zeroizing<[u8; KEY_LEN]>, ApiError> {
    cell.lock()
        .expect("vault lock")
        .clone()
        .ok_or_else(|| ApiError::new(ErrorCode::VaultLocked, "请先输入主密码解锁。"))
}

fn validate_master_password(password: &str) -> Result<(), ApiError> {
    if password.chars().count() < MASTER_PASSWORD_MIN_CHARS {
        return Err(ApiError::new(
            ErrorCode::InvalidMasterPassword,
            format!("主密码至少 {MASTER_PASSWORD_MIN_CHARS} 个字符。"),
        ));
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn vault_status(
    app: AppHandle,
    vault: State<'_, Vault>,
) -> Result<VaultStatus, ApiError> {
    let directory = config_directory(&app)?;
    let key = vault.key.clone();
    tauri::async_runtime::spawn_blocking(move || status_of(&directory, &key))
        .await
        .map_err(|error| ApiError::chain(ErrorCode::Internal, &error, "保险库任务失败"))?
}

#[tauri::command]
pub(crate) async fn vault_create(
    app: AppHandle,
    vault: State<'_, Vault>,
    password: String,
) -> Result<VaultStatus, ApiError> {
    let directory = config_directory(&app)?;
    let key = vault.key.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if load_vault(&directory)?.is_some() {
            return Err(ApiError::new(ErrorCode::VaultAlreadySetup, "主密码已设置。"));
        }
        validate_master_password(&password)?;
        let kdf = fresh_kdf()?;
        let derived = derive_key(&password, &kdf)?;
        let file = VaultFile {
            verifier: encrypt_b64(&derived, VERIFIER_PLAINTEXT)?,
            entries: BTreeMap::new(),
            kdf,
        };
        write_vault(&directory, &file)?;
        *key.lock().expect("vault lock") = Some(derived);
        status_of(&directory, &key)
    })
    .await
    .map_err(|error| ApiError::chain(ErrorCode::Internal, &error, "保险库任务失败"))?
}

#[tauri::command]
pub(crate) async fn vault_unlock(
    app: AppHandle,
    vault: State<'_, Vault>,
    password: String,
) -> Result<VaultStatus, ApiError> {
    let directory = config_directory(&app)?;
    let key = vault.key.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let file = load_vault(&directory)?
            .ok_or_else(|| ApiError::new(ErrorCode::VaultNotSetup, "尚未设置主密码。"))?;
        let derived = derive_key(&password, &file.kdf)?;
        decrypt_b64(&derived, &file.verifier)?;
        *key.lock().expect("vault lock") = Some(derived);
        status_of(&directory, &key)
    })
    .await
    .map_err(|error| ApiError::chain(ErrorCode::Internal, &error, "保险库任务失败"))?
}

#[tauri::command]
pub(crate) fn vault_lock(vault: State<'_, Vault>) {
    *vault.key.lock().expect("vault lock") = None;
}

#[tauri::command]
pub(crate) async fn vault_change_password(
    app: AppHandle,
    vault: State<'_, Vault>,
    old_password: String,
    new_password: String,
) -> Result<VaultStatus, ApiError> {
    let directory = config_directory(&app)?;
    let key = vault.key.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let file = load_vault(&directory)?
            .ok_or_else(|| ApiError::new(ErrorCode::VaultNotSetup, "尚未设置主密码。"))?;
        let old_key = derive_key(&old_password, &file.kdf)?;
        decrypt_b64(&old_key, &file.verifier)?;
        validate_master_password(&new_password)?;
        let kdf = fresh_kdf()?;
        let new_key = derive_key(&new_password, &kdf)?;
        let mut entries = BTreeMap::new();
        for (id, blob) in &file.entries {
            let plaintext = decrypt_b64(&old_key, blob)?;
            entries.insert(id.clone(), encrypt_b64(&new_key, &plaintext)?);
        }
        let file = VaultFile {
            verifier: encrypt_b64(&new_key, VERIFIER_PLAINTEXT)?,
            entries,
            kdf,
        };
        write_vault(&directory, &file)?;
        *key.lock().expect("vault lock") = Some(new_key);
        status_of(&directory, &key)
    })
    .await
    .map_err(|error| ApiError::chain(ErrorCode::Internal, &error, "保险库任务失败"))?
}

#[tauri::command]
pub(crate) async fn vault_clear(
    app: AppHandle,
    vault: State<'_, Vault>,
) -> Result<VaultStatus, ApiError> {
    let directory = config_directory(&app)?;
    let key = vault.key.clone();
    tauri::async_runtime::spawn_blocking(move || {
        *key.lock().expect("vault lock") = None;
        match fs::remove_file(directory.join(DATA_NAME)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(ApiError::chain(ErrorCode::IoError, &error, "无法删除保险库配置"));
            }
        }
        status_of(&directory, &key)
    })
    .await
    .map_err(|error| ApiError::chain(ErrorCode::Internal, &error, "保险库任务失败"))?
}

#[tauri::command]
pub(crate) async fn vault_set_password(
    app: AppHandle,
    vault: State<'_, Vault>,
    profile_id: u32,
    password: String,
) -> Result<VaultStatus, ApiError> {
    let directory = config_directory(&app)?;
    let key = vault.key.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let derived = unlocked_key(&key)?;
        let mut file = load_vault(&directory)?
            .ok_or_else(|| ApiError::new(ErrorCode::VaultNotSetup, "尚未设置主密码。"))?;
        file.entries.insert(
            profile_id.to_string(),
            encrypt_b64(&derived, password.as_bytes())?,
        );
        write_vault(&directory, &file)?;
        status_of(&directory, &key)
    })
    .await
    .map_err(|error| ApiError::chain(ErrorCode::Internal, &error, "保险库任务失败"))?
}

#[tauri::command]
pub(crate) async fn vault_remove_password(
    app: AppHandle,
    vault: State<'_, Vault>,
    profile_id: u32,
) -> Result<VaultStatus, ApiError> {
    let directory = config_directory(&app)?;
    let key = vault.key.clone();
    tauri::async_runtime::spawn_blocking(move || {
        unlocked_key(&key)?;
        let mut file = load_vault(&directory)?
            .ok_or_else(|| ApiError::new(ErrorCode::VaultNotSetup, "尚未设置主密码。"))?;
        if file.entries.remove(&profile_id.to_string()).is_some() {
            write_vault(&directory, &file)?;
        }
        status_of(&directory, &key)
    })
    .await
    .map_err(|error| ApiError::chain(ErrorCode::Internal, &error, "保险库任务失败"))?
}

#[tauri::command]
pub(crate) async fn vault_get_password(
    app: AppHandle,
    vault: State<'_, Vault>,
    profile_id: u32,
) -> Result<Option<String>, ApiError> {
    let directory = config_directory(&app)?;
    let key = vault.key.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let derived = unlocked_key(&key)?;
        let Some(file) = load_vault(&directory)? else {
            return Ok(None);
        };
        file.entries
            .get(&profile_id.to_string())
            .map(|blob| {
                decrypt_b64(&derived, blob)
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            })
            .transpose()
    })
    .await
    .map_err(|error| ApiError::chain(ErrorCode::Internal, &error, "保险库任务失败"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorCode;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    fn seed_vault(directory: &Path, password: &str) -> Zeroizing<[u8; KEY_LEN]> {
        let kdf = Kdf {
            salt: BASE64.encode(random_bytes(SALT_LEN).unwrap()),
            m_cost_kib: KDF_M_COST_KIB,
            t_cost: KDF_T_COST,
            p_cost: KDF_P_COST,
        };
        let key = derive_key(password, &kdf).unwrap();
        let file = VaultFile {
            verifier: encrypt_b64(&key, VERIFIER_PLAINTEXT).unwrap(),
            entries: BTreeMap::new(),
            kdf,
        };
        write_vault(directory, &file).unwrap();
        key
    }

    fn set_entry(key: &[u8; KEY_LEN], directory: &Path, profile_id: u32, password: &str) {
        let mut file = load_vault(directory).unwrap().unwrap();
        file.entries.insert(
            profile_id.to_string(),
            encrypt_b64(key, password.as_bytes()).unwrap(),
        );
        write_vault(directory, &file).unwrap();
    }

    fn read_entry(key: &[u8; KEY_LEN], directory: &Path, profile_id: u32) -> String {
        let file = load_vault(directory).unwrap().unwrap();
        String::from_utf8(
            decrypt_b64(key, &file.entries[&profile_id.to_string()]).unwrap(),
        )
        .unwrap()
    }

    fn unlock_in_memory(directory: &Path, password: &str) -> Result<Zeroizing<[u8; KEY_LEN]>, ApiError> {
        let file = load_vault(directory)?.expect("vault setup");
        let key = derive_key(password, &file.kdf)?;
        decrypt_b64(&key, &file.verifier)?;
        Ok(key)
    }

    #[test]
    fn verifier_rejects_wrong_master_password() {
        let directory = tempdir();
        seed_vault(directory.path(), "correct horse");
        assert!(unlock_in_memory(directory.path(), "correct horse").is_ok());
        let error = unlock_in_memory(directory.path(), "wrong horse").unwrap_err();
        assert_eq!(error.code, ErrorCode::VaultWrongPassword);
    }

    #[test]
    fn saved_passwords_roundtrip_and_survive_rotation() {
        let directory = tempdir();
        let key = seed_vault(directory.path(), "first master");
        set_entry(&key, directory.path(), 3, "alpha");
        set_entry(&key, directory.path(), 9, "beta");
        assert_eq!(read_entry(&key, directory.path(), 3), "alpha");
        assert_eq!(read_entry(&key, directory.path(), 9), "beta");

        // Re-encrypt every entry under a fresh key, like vault_change_password.
        let file = load_vault(directory.path()).unwrap().unwrap();
        let kdf = fresh_kdf().unwrap();
        let rotated = derive_key("second master", &kdf).unwrap();
        let mut entries = BTreeMap::new();
        for (id, blob) in &file.entries {
            let plaintext = decrypt_b64(&key, blob).unwrap();
            entries.insert(id.clone(), encrypt_b64(&rotated, &plaintext).unwrap());
        }
        write_vault(
            directory.path(),
            &VaultFile {
                verifier: encrypt_b64(&rotated, VERIFIER_PLAINTEXT).unwrap(),
                entries,
                kdf,
            },
        )
        .unwrap();

        assert!(unlock_in_memory(directory.path(), "first master").is_err());
        let rekeyed = unlock_in_memory(directory.path(), "second master").unwrap();
        assert_eq!(read_entry(&rekeyed, directory.path(), 9), "beta");
    }

    #[test]
    fn corrupt_vault_is_reported_and_never_overwritten() {
        let directory = tempdir();
        std::fs::write(directory.path().join(DATA_NAME), b"{ damaged").unwrap();
        let error = load_vault(directory.path()).unwrap_err();
        assert_eq!(error.code, ErrorCode::ConfigCorrupt);
        assert_eq!(
            std::fs::read_to_string(directory.path().join(DATA_NAME)).unwrap(),
            "{ damaged"
        );
    }

    #[test]
    fn short_master_passwords_are_rejected() {
        assert!(matches!(
            validate_master_password("short"),
            Err(error) if error.code == ErrorCode::InvalidMasterPassword
        ));
        assert!(validate_master_password("long enough").is_ok());
    }
}
