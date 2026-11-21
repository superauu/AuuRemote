//! Trusted RDP host certificates (TOFU): hosts whose presented certificate
//! fingerprint the user accepted are remembered, so later connections skip
//! the manual verification prompt.

use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};

use crate::{
    errors::ApiError,
    store::{read_json_file, with_file_lock, write_json_file},
};

const DATA_NAME: &str = "trusted_hosts.json";
const LOCK_NAME: &str = "trusted_hosts.lock";

/// host (as validated in the connection) -> certificate SHA-256 fingerprint
/// in `AA:BB:…` form.
pub(crate) type TrustMap = BTreeMap<String, String>;

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrustFile {
    hosts: TrustMap,
}

pub(crate) fn load(directory: &Path) -> Result<TrustMap, ApiError> {
    Ok(read_json_file::<TrustFile>(&directory.join(DATA_NAME))?
        .unwrap_or_default()
        .hosts)
}

pub(crate) fn trust(directory: &Path, host: String, fingerprint: String) -> Result<(), ApiError> {
    with_file_lock(directory, LOCK_NAME, || {
        let mut file = read_json_file::<TrustFile>(&directory.join(DATA_NAME))?.unwrap_or_default();
        file.hosts.insert(host, fingerprint);
        write_json_file(&directory.join(DATA_NAME), &file)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorCode;

    #[test]
    fn trust_persists_and_survives_reloads() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        trust(dir.path(), "workstation".into(), "AA:BB".into())?;
        trust(dir.path(), "10.0.0.5".into(), "CC:DD".into())?;
        let map = load(dir.path())?;
        assert_eq!(map.get("workstation").map(|value| value.as_str()), Some("AA:BB"));
        assert_eq!(map.get("10.0.0.5").map(|value| value.as_str()), Some("CC:DD"));

        // Overwriting an existing host keeps a single entry.
        trust(dir.path(), "workstation".into(), "EE:FF".into())?;
        let map = load(dir.path())?;
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("workstation").map(|value| value.as_str()), Some("EE:FF"));
        Ok(())
    }

    #[test]
    fn corrupt_trust_file_is_reported_and_never_overwritten() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join(DATA_NAME), b"{ damaged")?;
        let error = load(dir.path()).unwrap_err();
        assert_eq!(error.code, ErrorCode::ConfigCorrupt);
        assert_eq!(
            std::fs::read_to_string(dir.path().join(DATA_NAME))?,
            "{ damaged"
        );
        Ok(())
    }
}
