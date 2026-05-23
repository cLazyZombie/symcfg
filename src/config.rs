use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CURRENT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkEntry {
    pub link: PathBuf,
    pub src: PathBuf,
}

impl LinkEntry {
    pub fn new(link: impl Into<PathBuf>, src: impl Into<PathBuf>) -> Self {
        Self {
            link: link.into(),
            src: src.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigFile {
    pub version: u32,
    pub links: Vec<LinkEntry>,
}

impl Default for ConfigFile {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            links: Vec::new(),
        }
    }
}

impl ConfigFile {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.version != CURRENT_VERSION {
            return Err(ConfigError::UnsupportedVersion {
                version: self.version,
            });
        }

        for entry in &self.links {
            validate_entry(entry)?;
        }

        Ok(())
    }

    pub fn merge_entry(&mut self, entry: LinkEntry) -> Result<MergeStatus, ConfigError> {
        validate_entry(&entry)?;

        match self
            .links
            .iter()
            .find(|existing| existing.link == entry.link)
        {
            Some(existing) if existing.src == entry.src => Ok(MergeStatus::Duplicate),
            Some(existing) => Err(ConfigError::Conflict {
                link: entry.link,
                existing_src: existing.src.clone(),
                new_src: entry.src,
            }),
            None => {
                self.links.push(entry);
                self.sort_links();
                Ok(MergeStatus::Added)
            }
        }
    }

    pub fn save(&mut self, path: &Path) -> Result<(), ConfigError> {
        self.validate()?;
        self.sort_links();

        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|err| {
                // LCOV_EXCL_START
                ConfigError::Io {
                    path: parent.to_path_buf(),
                    kind: err.kind(),
                    message: err.to_string(),
                }
            })?;
            // LCOV_EXCL_STOP
        }

        let mut json = serde_json::to_string_pretty(self).map_err(|err| {
            // LCOV_EXCL_START
            ConfigError::Json {
                path: path.to_path_buf(),
                message: err.to_string(),
            }
        })?;
        // LCOV_EXCL_STOP
        json.push('\n');

        fs::write(path, json).map_err(|err| ConfigError::Io {
            path: path.to_path_buf(),
            kind: err.kind(),
            message: err.to_string(),
        })
    }

    pub fn load(path: &Path) -> Result<ConfigFile, ConfigError> {
        let contents = fs::read_to_string(path).map_err(|err| ConfigError::Io {
            path: path.to_path_buf(),
            kind: err.kind(),
            message: err.to_string(),
        })?;

        let mut config: ConfigFile =
            serde_json::from_str(&contents).map_err(|err| ConfigError::Json {
                path: path.to_path_buf(),
                message: err.to_string(),
            })?;
        config.validate()?;
        config.sort_links();
        Ok(config)
    }

    pub fn load_or_default(path: &Path) -> Result<ConfigFile, ConfigError> {
        match Self::load(path) {
            Ok(config) => Ok(config),
            Err(ConfigError::Io {
                kind: ErrorKind::NotFound,
                ..
            }) => Ok(Self::default()),
            Err(err) => Err(err),
        }
    }

    fn sort_links(&mut self) {
        self.links.sort_by(|left, right| left.link.cmp(&right.link));
    }
}

fn validate_entry(entry: &LinkEntry) -> Result<(), ConfigError> {
    if entry.link.as_os_str().is_empty() {
        return Err(ConfigError::EmptyLink);
    }

    if entry.src.as_os_str().is_empty() {
        return Err(ConfigError::EmptySrc);
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStatus {
    Added,
    Duplicate,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("unsupported config version {version}")]
    UnsupportedVersion { version: u32 },

    #[error("link must not be empty")]
    EmptyLink,

    #[error("src must not be empty")]
    EmptySrc,

    #[error("link {link:?} already points to {existing_src:?}, not {new_src:?}")]
    Conflict {
        link: PathBuf,
        existing_src: PathBuf,
        new_src: PathBuf,
    },

    #[error("I/O error at {path:?}: {message}")]
    Io {
        path: PathBuf,
        kind: ErrorKind,
        message: String,
    },

    #[error("JSON error at {path:?}: {message}")]
    Json { path: PathBuf, message: String },
}

// LCOV_EXCL_START
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_current_version_and_has_no_links() {
        let config = ConfigFile::default();

        assert_eq!(CURRENT_VERSION, 1);
        assert_eq!(config.version, CURRENT_VERSION);
        assert!(config.links.is_empty());
    }

    #[test]
    fn save_writes_stable_json_sorted_by_link_with_src_field_name() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let path = dir.path().join("symbolic.json");
        let mut config = ConfigFile {
            version: CURRENT_VERSION,
            links: vec![
                LinkEntry::new("links/zeta", "sources/zeta"),
                LinkEntry::new("links/alpha", "sources/alpha"),
            ],
        };

        config.save(&path).expect("save config");

        let actual = std::fs::read_to_string(&path).expect("read saved config");
        let expected = concat!(
            "{\n",
            "  \"version\": 1,\n",
            "  \"links\": [\n",
            "    {\n",
            "      \"link\": \"links/alpha\",\n",
            "      \"src\": \"sources/alpha\"\n",
            "    },\n",
            "    {\n",
            "      \"link\": \"links/zeta\",\n",
            "      \"src\": \"sources/zeta\"\n",
            "    }\n",
            "  ]\n",
            "}\n",
        );
        assert_eq!(actual, expected);
        assert!(actual.contains("\"src\""));
        assert!(!actual.contains("target"));
    }

    #[test]
    fn merging_same_link_and_src_twice_reports_duplicate_without_adding_entry() {
        let entry = LinkEntry::new("links/tool", "sources/tool");
        let mut config = ConfigFile::default();

        let first = config
            .merge_entry(entry.clone())
            .expect("first merge should add entry");
        let second = config
            .merge_entry(entry)
            .expect("second merge should be recognized as duplicate");

        assert_eq!(first, MergeStatus::Added);
        assert_eq!(second, MergeStatus::Duplicate);
        assert_eq!(
            config.links,
            vec![LinkEntry::new("links/tool", "sources/tool")]
        );
    }

    #[test]
    fn merging_same_link_with_different_src_reports_conflict_and_keeps_original_entry() {
        let mut config = ConfigFile::default();
        let original = LinkEntry::new("links/tool", "sources/tool-v1");
        let conflicting = LinkEntry::new("links/tool", "sources/tool-v2");

        let first = config
            .merge_entry(original.clone())
            .expect("first merge should add entry");
        let err = config
            .merge_entry(conflicting.clone())
            .expect_err("same link with a different src should conflict");

        assert_eq!(first, MergeStatus::Added);
        assert_eq!(
            err,
            ConfigError::Conflict {
                link: conflicting.link.clone(),
                existing_src: original.src.clone(),
                new_src: conflicting.src.clone(),
            }
        );
        assert_eq!(config.links, vec![original]);
    }

    #[test]
    fn validation_rejects_unsupported_version_and_empty_link_or_src() {
        let unsupported_version = ConfigFile {
            version: CURRENT_VERSION + 1,
            links: Vec::new(),
        };
        assert!(unsupported_version.validate().is_err());

        let empty_link = ConfigFile {
            version: CURRENT_VERSION,
            links: vec![LinkEntry::new("", "sources/tool")],
        };
        assert!(empty_link.validate().is_err());

        let empty_src = ConfigFile {
            version: CURRENT_VERSION,
            links: vec![LinkEntry::new("links/tool", "")],
        };
        assert!(empty_src.validate().is_err());
    }

    #[test]
    fn loading_saved_config_round_trips() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let path = dir.path().join("symbolic.json");
        let mut original = ConfigFile {
            version: CURRENT_VERSION,
            links: vec![
                LinkEntry::new("links/beta", "sources/beta"),
                LinkEntry::new("links/alpha", "sources/alpha"),
            ],
        };

        original.save(&path).expect("save config");
        let loaded = ConfigFile::load(&path).expect("load config");

        assert_eq!(loaded.version, CURRENT_VERSION);
        assert_eq!(
            loaded.links,
            vec![
                LinkEntry::new("links/alpha", "sources/alpha"),
                LinkEntry::new("links/beta", "sources/beta"),
            ]
        );
    }
    #[test]
    fn load_or_default_returns_error_for_existing_malformed_config() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let path = dir.path().join("symbolic.json");
        std::fs::write(&path, "{ invalid json\n").expect("write malformed config");

        let err = ConfigFile::load_or_default(&path)
            .expect_err("existing malformed config should not default");

        match err {
            ConfigError::Json {
                path: err_path,
                message,
            } => {
                assert_eq!(err_path, path);
                assert!(!message.is_empty());
            }
            other => panic!("expected JSON error, got {other:?}"),
        }
    }
}
// LCOV_EXCL_STOP
