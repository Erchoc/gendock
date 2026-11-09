use crate::{models::ModelError, paths::Paths};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs::OpenOptions, io::Write};

#[derive(Clone, Serialize, Deserialize)]
pub struct Principal {
    pub name: String,
    pub admin: bool,
}
#[derive(Serialize, Deserialize)]
struct Credential {
    token: String,
    admin: bool,
}
fn file(paths: &Paths) -> std::path::PathBuf {
    paths.config.join("server-tokens.json")
}
fn credentials(paths: &Paths) -> Result<BTreeMap<String, Credential>> {
    let path = file(paths);
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}
pub fn token(paths: &Paths, name: &str) -> Result<String> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err(ModelError {
            code: "invalid_client_name",
            message: "Client names must contain 1–64 letters, digits, underscores or hyphens.",
            retryable: false,
            next_step: "Choose a valid unique client name.",
            details: json!({}),
        }
        .into());
    }
    std::fs::create_dir_all(&paths.config)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(paths.config.join("server-tokens.lock"))?;
    fs2::FileExt::lock_exclusive(&lock)?;
    let mut data = credentials(paths)?;
    if let Some(value) = data.get(name) {
        return Ok(value.token.clone());
    }
    let value = format!(
        "gd_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    data.insert(
        name.into(),
        Credential {
            token: value.clone(),
            admin: name == "admin",
        },
    );
    let mut output = tempfile::NamedTempFile::new_in(&paths.config)?;
    output.write_all(&serde_json::to_vec_pretty(&data)?)?;
    output.as_file().sync_all()?;
    output.persist(file(paths))?;
    Ok(value)
}
pub fn authenticate(paths: &Paths, value: &str) -> Result<Option<Principal>> {
    let supplied = Sha256::digest(value.as_bytes());
    for (name, entry) in credentials(paths)? {
        let expected = Sha256::digest(entry.token.as_bytes());
        let different = supplied
            .iter()
            .zip(expected)
            .fold(0u8, |value, (a, b)| value | (a ^ b));
        if different == 0 {
            return Ok(Some(Principal {
                name,
                admin: entry.admin,
            }));
        }
    }
    Ok(None)
}
pub fn owner_directory(name: &str) -> String {
    format!("{:x}", Sha256::digest(name.as_bytes()))
}
