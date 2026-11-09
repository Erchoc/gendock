use std::{env, path::PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

/// XDG locations, including on macOS. Relative XDG overrides are ignored.
#[derive(Debug, Clone, Serialize)]
pub struct Paths {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self> {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .context("HOME must be an absolute path")?;
        Ok(Self::resolve(&home, |key| {
            env::var_os(key).map(PathBuf::from)
        }))
    }

    pub fn resolve(home: &std::path::Path, get: impl Fn(&str) -> Option<PathBuf>) -> Self {
        let root = |key, fallback: &str| {
            get(key)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(fallback))
                .join("gendock")
        };
        Self {
            config: root("XDG_CONFIG_HOME", ".config"),
            data: root("XDG_DATA_HOME", ".local/share"),
            cache: root("XDG_CACHE_HOME", ".cache"),
            state: root("XDG_STATE_HOME", ".local/state"),
        }
    }

    pub fn database(&self) -> PathBuf {
        self.data.join("database/gendock.sqlite3")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_overrides_and_relative_fallbacks() {
        let paths = Paths::resolve(std::path::Path::new("/users/test"), |key| match key {
            "XDG_DATA_HOME" => Some("/external/data".into()),
            "XDG_CONFIG_HOME" => Some("relative".into()),
            _ => None,
        });
        assert_eq!(paths.data, PathBuf::from("/external/data/gendock"));
        assert_eq!(paths.config, PathBuf::from("/users/test/.config/gendock"));
        assert_eq!(
            paths.state,
            PathBuf::from("/users/test/.local/state/gendock")
        );
    }
}
