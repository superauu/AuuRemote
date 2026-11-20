use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufReader},
    path::Path,
};

use serde::{Serialize, de::DeserializeOwned};
use tauri::{AppHandle, Manager};

use crate::errors::{ApiError, ErrorCode};

/// Runs `operation` while holding an advisory lock in `directory`, so reads and
/// modifications serialize across app instances. The lock is separate from the
/// data files, which are replaced atomically after each write.
pub(crate) fn with_file_lock<T>(
    directory: &Path,
    lock_name: &str,
    operation: impl FnOnce() -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    fs::create_dir_all(directory)
        .map_err(|error| ApiError::chain(ErrorCode::IoError, &error, "无法创建配置目录"))?;

    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join(lock_name))
        .map_err(|error| ApiError::chain(ErrorCode::IoError, &error, "无法打开配置锁"))?;
    lock.lock()
        .map_err(|error| ApiError::chain(ErrorCode::IoError, &error, "无法锁定配置文件"))?;

    operation()
}

/// Reads a JSON file, returning `None` when it does not exist yet. A damaged
/// file surfaces as `configCorrupt`; callers must never overwrite it.
pub(crate) fn read_json_file<S: DeserializeOwned>(path: &Path) -> Result<Option<S>, ApiError> {
    match File::open(path) {
        Ok(file) => serde_json::from_reader(BufReader::new(file))
            .map(Some)
            .map_err(|error| ApiError::chain(ErrorCode::ConfigCorrupt, &error, "配置文件损坏")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ApiError::chain(ErrorCode::IoError, &error, "无法打开配置文件")),
    }
}

/// Atomically replaces a JSON file: write a temp file, sync, then rename over
/// the original.
pub(crate) fn write_json_file<S: Serialize>(path: &Path, data: &S) -> Result<(), ApiError> {
    let directory = path
        .parent()
        .ok_or_else(|| ApiError::new(ErrorCode::IoError, "配置路径缺少父目录"))?;
    let mut temp = tempfile::NamedTempFile::new_in(directory)
        .map_err(|error| ApiError::chain(ErrorCode::IoError, &error, "无法创建配置临时文件"))?;
    serde_json::to_writer_pretty(temp.as_file_mut(), data)
        .map_err(|error| ApiError::chain(ErrorCode::IoError, &error, "无法写入配置文件"))?;
    temp.as_file()
        .sync_all()
        .map_err(|error| ApiError::chain(ErrorCode::IoError, &error, "无法同步配置文件"))?;
    temp.persist(path)
        .map_err(|error| ApiError::chain(ErrorCode::IoError, &error, "无法替换配置文件"))?;
    Ok(())
}

pub(crate) fn config_directory(app: &AppHandle) -> Result<std::path::PathBuf, ApiError> {
    app.path()
        .app_config_dir()
        .map_err(|error| ApiError::chain(ErrorCode::IoError, &error, "无法找到应用配置目录"))
}
