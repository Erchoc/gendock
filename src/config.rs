//! User-editable defaults. Never silently repair or overwrite invalid configuration.
use crate::{models::ModelError, paths::Paths};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    io::{Read, Write},
    path::Path,
};

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub defaults: Defaults,
    pub server: ServerConfig,
    pub remote: RemoteConfig,
    pub output: OutputConfig,
    pub power: PowerConfig,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Defaults {
    pub image_generate: String,
    pub image_edit: String,
}
impl Default for Defaults {
    fn default() -> Self {
        Self {
            image_generate: crate::image::DEFAULT_MODEL.into(),
            image_edit: crate::image::DEFAULT_MODEL.into(),
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub max_pending_per_user: u32,
}
impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 9966,
            max_pending_per_user: 20,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PowerConfig {
    pub prevent_sleep: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RemoteConfig {
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
}
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OutputConfig {
    pub image: Option<std::path::PathBuf>,
    pub image_edit: Option<std::path::PathBuf>,
    pub video: Option<std::path::PathBuf>,
    pub audio: Option<std::path::PathBuf>,
    pub music: Option<std::path::PathBuf>,
}

impl OutputConfig {
    pub fn image_directory(&self, editing: bool, cwd: &Path) -> std::path::PathBuf {
        if editing {
            self.image_edit.as_ref().or(self.image.as_ref())
        } else {
            self.image.as_ref()
        }
        .map_or_else(|| cwd.to_path_buf(), Clone::clone)
    }
}

impl Config {
    pub fn load_or_init(paths: &Paths) -> Result<Self> {
        let path = paths.config.join("config.toml");
        if !path.exists() {
            std::fs::create_dir_all(&paths.config)?;
            let mut temp = tempfile::NamedTempFile::new_in(&paths.config)?;
            temp.write_all(
                b"# Default models; --model overrides these values for one invocation.\n",
            )?;
            temp.write_all(toml::to_string_pretty(&Self::default())?.as_bytes())?;
            temp.as_file().sync_all()?;
            if let Err(error) = temp.persist_noclobber(&path)
                && error.error.kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(error.error.into());
            }
        }
        Self::read(&path)
    }
    fn read(path: &Path) -> Result<Self> {
        let mut text = String::new();
        std::fs::File::open(path)?
            .take(65537)
            .read_to_string(&mut text)?;
        let invalid = || ModelError {
            code: "invalid_config",
            message: "The configuration file is invalid.",
            retryable: false,
            next_step: "Inspect docs/configuration.md and correct the TOML file. Use nonempty model IDs, absolute export directories and valid HTTP(S) origins; unknown keys are rejected.",
            details: json!({"config_path":path}),
        };
        if text.len() > 65536 {
            return Err(invalid().into());
        }
        let config: Self = toml::from_str(&text).map_err(|_| invalid())?;
        config.validate().map_err(|_| invalid())?;
        Ok(config)
    }
    fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            !self.defaults.image_generate.trim().is_empty()
                && !self.defaults.image_edit.trim().is_empty()
                && (1..=1000).contains(&self.server.max_pending_per_user)
                && self.server.host.parse::<std::net::IpAddr>().is_ok(),
            "invalid defaults or server settings"
        );
        if let Some(endpoint) = &self.remote.endpoint {
            let url = reqwest::Url::parse(endpoint)?;
            anyhow::ensure!(
                matches!(url.scheme(), "http" | "https")
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.path() == "/"
                    && url.query().is_none()
                    && url.fragment().is_none(),
                "invalid endpoint"
            );
        }
        if let Some(key) = &self.remote.api_key {
            anyhow::ensure!(
                !key.trim().is_empty() && !key.chars().any(char::is_control),
                "invalid API key"
            );
        }
        for path in [
            &self.output.image,
            &self.output.image_edit,
            &self.output.video,
            &self.output.audio,
            &self.output.music,
        ]
        .into_iter()
        .flatten()
        {
            anyhow::ensure!(path.is_absolute(), "output directories must be absolute");
        }
        Ok(())
    }
}

fn config_error(path: &Path) -> anyhow::Error {
    ModelError {
        code: "invalid_config",
        message: "The configuration key or value is invalid.",
        retryable: false,
        next_step: "Use gendock config list to inspect supported settings. Output directories must be absolute and endpoints must be HTTP(S) URLs without credentials, query or fragment.",
        details: json!({"config_path": path}),
    }.into()
}

/// Configuration administration never opens the database or contacts a remote service.
pub fn execute(paths: &Paths, command: crate::cli::ConfigCommand) -> Result<serde_json::Value> {
    use crate::cli::ConfigCommand;
    let path = paths.config.join("config.toml");
    if matches!(command, ConfigCommand::Path) {
        return Ok(json!({"config_path": path}));
    }
    std::fs::create_dir_all(&paths.config)?;
    // A stable sidecar lock survives atomic replacement of config.toml.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(paths.config.join("config.lock"))?;
    fs2::FileExt::lock_exclusive(&lock)?;
    let config = Config::load_or_init(paths)?;
    let mut effective = serde_json::to_value(&config)?;
    let (key, value) = match command {
        ConfigCommand::List => {
            redact(&mut effective);
            return Ok(json!({"config_path": path, "settings": effective}));
        }
        ConfigCommand::Get { key } => {
            let (section, field) = key.split_once('.').ok_or_else(|| config_error(&path))?;
            let value = effective
                .get(section)
                .and_then(|s| s.get(field))
                .ok_or_else(|| config_error(&path))?;
            return Ok(
                json!({"key": key, "value": if key == "remote.api_key" && !value.is_null() { json!("[REDACTED]") } else { value.clone() }}),
            );
        }
        ConfigCommand::Set { key, value } => (key, Some(value)),
        ConfigCommand::Unset { key } => (key, None),
        ConfigCommand::Path => unreachable!(),
    };
    let (section, field) = key.split_once('.').ok_or_else(|| config_error(&path))?;
    let current = effective
        .get(section)
        .and_then(|s| s.get(field))
        .ok_or_else(|| config_error(&path))?;
    let mut document: toml::Value = toml::from_str(&std::fs::read_to_string(&path)?)?;
    if let Some(value) = value {
        let value = if current.is_boolean() {
            toml::Value::Boolean(value.parse().map_err(|_| config_error(&path))?)
        } else if current.is_number() {
            toml::Value::Integer(value.parse().map_err(|_| config_error(&path))?)
        } else {
            toml::Value::String(value)
        };
        let table = document
            .as_table_mut()
            .ok_or_else(|| config_error(&path))?
            .entry(section)
            .or_insert_with(|| toml::Value::Table(Default::default()))
            .as_table_mut()
            .ok_or_else(|| config_error(&path))?;
        table.insert(field.into(), value);
    } else if let Some(table) = document
        .get_mut(section)
        .and_then(toml::Value::as_table_mut)
    {
        table.remove(field);
    }
    let text = toml::to_string_pretty(&document)?;
    let updated: Config = toml::from_str(&text).map_err(|_| config_error(&path))?;
    updated.validate().map_err(|_| config_error(&path))?;
    if text.len() > 65536 {
        return Err(config_error(&path));
    }
    let mut temp = tempfile::NamedTempFile::new_in(&paths.config)?;
    temp.write_all(text.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(&path)?;
    std::fs::File::open(&paths.config)?.sync_all()?;
    effective = serde_json::to_value(updated)?;
    redact(&mut effective);
    Ok(json!({"config_path": path, "settings": effective, "updated_key": key}))
}
fn redact(value: &mut serde_json::Value) {
    if !value["remote"]["api_key"].is_null() {
        value["remote"]["api_key"] = json!("[REDACTED]");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn administration_validates_redacts_and_restores_defaults() {
        use crate::cli::ConfigCommand::*;
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(temp.path(), |_| None);
        execute(&paths, Path).unwrap();
        assert!(!paths.config.exists());
        execute(
            &paths,
            Set {
                key: "remote.api_key".into(),
                value: "private-token".into(),
            },
        )
        .unwrap();
        assert!(
            !execute(&paths, List)
                .unwrap()
                .to_string()
                .contains("private-token")
        );
        assert_eq!(
            execute(
                &paths,
                Get {
                    key: "remote.api_key".into()
                }
            )
            .unwrap()["value"],
            "[REDACTED]"
        );
        let file = paths.config.join("config.toml");
        let previous = std::fs::read_to_string(&file).unwrap();
        for (key, value) in [
            ("output.image", "relative"),
            ("remote.endpoint", "https://user:secret@example.com"),
            ("defaults.image_edit", ""),
            ("server.port", "99999"),
            ("unknown.key", "x"),
        ] {
            assert!(
                execute(
                    &paths,
                    Set {
                        key: key.into(),
                        value: value.into()
                    }
                )
                .is_err()
            );
            assert_eq!(std::fs::read_to_string(&file).unwrap(), previous);
        }
        execute(
            &paths,
            Set {
                key: "output.image".into(),
                value: temp.path().join("exports").to_str().unwrap().into(),
            },
        )
        .unwrap();
        let cfg = Config::load_or_init(&paths).unwrap();
        assert_eq!(
            cfg.output.image_directory(true, temp.path()),
            temp.path().join("exports")
        );
        execute(
            &paths,
            Unset {
                key: "output.image".into(),
            },
        )
        .unwrap();
        assert_eq!(
            Config::load_or_init(&paths)
                .unwrap()
                .output
                .image_directory(false, temp.path()),
            temp.path()
        );
        execute(
            &paths,
            Set {
                key: "defaults.image_edit".into(),
                value: "custom".into(),
            },
        )
        .unwrap();
        execute(
            &paths,
            Unset {
                key: "defaults.image_edit".into(),
            },
        )
        .unwrap();
        assert_eq!(
            Config::load_or_init(&paths).unwrap().defaults.image_edit,
            crate::image::DEFAULT_MODEL
        );
    }
    #[test]
    fn concurrent_updates_preserve_independent_settings() {
        use crate::cli::ConfigCommand::Set;
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(temp.path(), |_| None);
        std::thread::scope(|scope| {
            let a = scope.spawn(|| {
                execute(
                    &paths,
                    Set {
                        key: "defaults.image_generate".into(),
                        value: "generate-custom".into(),
                    },
                )
                .unwrap()
            });
            let b = scope.spawn(|| {
                execute(
                    &paths,
                    Set {
                        key: "defaults.image_edit".into(),
                        value: "edit-custom".into(),
                    },
                )
                .unwrap()
            });
            a.join().unwrap();
            b.join().unwrap();
        });
        let cfg = Config::load_or_init(&paths).unwrap();
        assert_eq!(cfg.defaults.image_generate, "generate-custom");
        assert_eq!(cfg.defaults.image_edit, "edit-custom");
    }
    #[test]
    fn creates_defaults_and_preserves_user_edits() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(temp.path(), |_| None);
        assert_eq!(
            Config::load_or_init(&paths)
                .unwrap()
                .defaults
                .image_generate,
            "flux2-klein-4b-q4"
        );
        let path = paths.config.join("config.toml");
        let contents = "[defaults]\nimage_generate = 'custom-image'\nimage_edit = 'custom-edit'\n";
        std::fs::write(&path, contents).unwrap();
        let config = Config::load_or_init(&paths).unwrap();
        assert_eq!(config.defaults.image_generate, "custom-image");
        assert_eq!(config.defaults.image_edit, "custom-edit");
        assert_eq!(std::fs::read_to_string(path).unwrap(), contents);
    }
    #[test]
    fn concurrent_initialization_creates_one_valid_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(temp.path(), |_| None);
        std::thread::scope(|scope| {
            let handles = (0..4)
                .map(|_| scope.spawn(|| Config::load_or_init(&paths).unwrap()))
                .collect::<Vec<_>>();
            for handle in handles {
                assert_eq!(
                    handle.join().unwrap().defaults.image_edit,
                    crate::image::DEFAULT_MODEL
                );
            }
        });
        assert!(Config::load_or_init(&paths).is_ok());
    }
    #[test]
    fn invalid_configuration_is_not_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(temp.path(), |_| None);
        std::fs::create_dir_all(&paths.config).unwrap();
        for text in [
            "[defaults]\nimage_generete = 'typo'\n",
            "[defaults]\nimage_edit = ''\n",
            "not valid [",
        ] {
            std::fs::write(paths.config.join("config.toml"), text).unwrap();
            let error = Config::load_or_init(&paths).unwrap_err();
            assert_eq!(
                error.downcast_ref::<ModelError>().unwrap().code,
                "invalid_config"
            );
            assert_eq!(
                std::fs::read_to_string(paths.config.join("config.toml")).unwrap(),
                text
            );
        }
    }
}
