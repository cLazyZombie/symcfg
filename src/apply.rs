use std::{
    fs,
    io::{self, ErrorKind},
    os::unix::fs as unix_fs,
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::{
    config::{ConfigError, ConfigFile, LinkEntry},
    paths,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyDecision {
    Create,
    Skip,
}

pub trait ApplyPrompter {
    fn decide_create_link(&mut self, entry: &LinkEntry) -> Result<ApplyDecision, ApplyError>;
}

#[derive(Debug)]
pub struct ApplyOptions<P> {
    pub yes: bool,
    pub prompter: P,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ApplyReport {
    pub created: usize,
    pub skipped: usize,
    pub conflicts: Vec<ApplyConflict>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ApplyConflict {
    pub link: PathBuf,
    pub src: PathBuf,
}

#[derive(Debug, Error)]
pub enum ApplyError {
    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error("I/O error at {path:?}: {source}")]
    Io { path: PathBuf, source: io::Error },

    #[error("prompt failed: {message}")]
    Prompt { message: String },
}

pub fn validate_config_file(config_path: &Path) -> Result<ConfigFile, ApplyError> {
    Ok(ConfigFile::load(config_path)?)
}

pub fn apply_config<P: ApplyPrompter>(
    config_path: &Path,
    options: ApplyOptions<P>,
) -> Result<ApplyReport, ApplyError> {
    let config = validate_config_file(config_path)?;
    let ApplyOptions { yes, mut prompter } = options;
    let mut report = ApplyReport {
        created: 0,
        skipped: 0,
        conflicts: Vec::new(),
    };

    for entry in &config.links {
        let link = paths::absolute_lexical(&entry.link).map_err(
            // LCOV_EXCL_START
            |source| ApplyError::Io {
                path: entry.link.clone(),
                source,
            },
        )?;
        // LCOV_EXCL_STOP
        let src = paths::absolute_lexical(&entry.src).map_err(
            // LCOV_EXCL_START
            |source| ApplyError::Io {
                path: entry.src.clone(),
                source,
            },
        )?;
        // LCOV_EXCL_STOP

        match fs::symlink_metadata(&link) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    let target = fs::read_link(&link).map_err(
                        // LCOV_EXCL_START
                        |source| ApplyError::Io {
                            path: link.clone(),
                            source,
                        },
                    )?;
                    // LCOV_EXCL_STOP
                    let target = paths::resolve_symlink_target_lexical(&link, &target).map_err(
                        // LCOV_EXCL_START
                        |source| ApplyError::Io {
                            path: link.clone(),
                            source,
                        },
                    )?;
                    // LCOV_EXCL_STOP

                    if target == src {
                        report.skipped += 1;
                    } else {
                        report.conflicts.push(ApplyConflict {
                            link: link.clone(),
                            src: src.clone(),
                        });
                    }
                } else {
                    report.conflicts.push(ApplyConflict {
                        link: link.clone(),
                        src: src.clone(),
                    });
                }
            }
            Err(err) if err.kind() == ErrorKind::NotFound => {
                if !link_parent_exists(&link) {
                    report.skipped += 1;
                    continue;
                }

                let decision = if yes {
                    ApplyDecision::Create
                } else {
                    prompter.decide_create_link(entry)?
                };

                match decision {
                    ApplyDecision::Create => {
                        unix_fs::symlink(&src, &link).map_err(
                            // LCOV_EXCL_START
                            |source| ApplyError::Io {
                                path: link.clone(),
                                source,
                            },
                        )?;
                        // LCOV_EXCL_STOP
                        report.created += 1;
                    }
                    ApplyDecision::Skip => {
                        report.skipped += 1;
                    }
                }
            }
            // LCOV_EXCL_START
            Err(source) => {
                return Err(ApplyError::Io {
                    path: link.clone(),
                    source,
                });
                // LCOV_EXCL_STOP
            }
        }
    }

    Ok(report)
}

fn link_parent_exists(link: &Path) -> bool {
    link.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .is_none_or(Path::exists)
}

// LCOV_EXCL_START
#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        fs,
        os::unix::fs as unix_fs,
        path::{Path, PathBuf},
        rc::Rc,
    };

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::config::CURRENT_VERSION;

    #[derive(Debug)]
    struct RecordingPrompter {
        decision: ApplyDecision,
        calls: Rc<Cell<usize>>,
    }

    impl RecordingPrompter {
        fn new(decision: ApplyDecision, calls: Rc<Cell<usize>>) -> Self {
            Self { decision, calls }
        }
    }

    impl ApplyPrompter for RecordingPrompter {
        fn decide_create_link(&mut self, _entry: &LinkEntry) -> Result<ApplyDecision, ApplyError> {
            self.calls.set(self.calls.get() + 1);
            Ok(self.decision)
        }
    }

    fn write_config(path: &Path, links: Vec<LinkEntry>) {
        let config = ConfigFile {
            version: CURRENT_VERSION,
            links,
        };
        let json = serde_json::to_string_pretty(&config).expect("serialize config");
        fs::write(path, json).expect("write config");
    }

    fn write_config_with_version(path: &Path, version: u32, links: Vec<LinkEntry>) {
        let links = links
            .into_iter()
            .map(|entry| json!({ "link": entry.link, "src": entry.src }))
            .collect::<Vec<_>>();
        let json = json!({ "version": version, "links": links });
        fs::write(
            path,
            serde_json::to_string_pretty(&json).expect("serialize config"),
        )
        .expect("write config");
    }

    fn source_file(root: &TempDir, name: &str) -> PathBuf {
        let path = root.path().join(name);
        fs::write(&path, name).expect("write source file");
        path
    }

    fn assert_symlink_to(link: &Path, expected_src: &Path) {
        let metadata = fs::symlink_metadata(link).expect("link metadata");
        assert!(metadata.file_type().is_symlink());
        assert_eq!(fs::read_link(link).expect("read symlink"), expected_src);
    }

    #[test]
    fn validate_config_file_loads_valid_config_and_rejects_invalid_version() {
        let temp = TempDir::new().expect("tempdir");
        let valid_config_path = temp.path().join("valid.json");
        let invalid_config_path = temp.path().join("invalid.json");
        let src = source_file(&temp, "source.txt");
        let link = temp.path().join("link.txt");

        write_config(&valid_config_path, vec![LinkEntry::new(&link, &src)]);
        write_config_with_version(
            &invalid_config_path,
            CURRENT_VERSION + 1,
            vec![LinkEntry::new(&link, &src)],
        );

        let config = validate_config_file(&valid_config_path).expect("valid config loads");
        assert_eq!(config.links, vec![LinkEntry::new(&link, &src)]);

        assert!(matches!(
            validate_config_file(&invalid_config_path),
            Err(ApplyError::Config(ConfigError::UnsupportedVersion { version }))
                if version == CURRENT_VERSION + 1
        ));
    }

    #[test]
    fn apply_config_skips_already_correct_symlink() {
        let temp = TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.json");
        let src = source_file(&temp, "source.txt");
        let link = temp.path().join("link.txt");
        unix_fs::symlink(&src, &link).expect("create existing symlink");
        write_config(&config_path, vec![LinkEntry::new(&link, &src)]);

        let report = apply_config(
            &config_path,
            ApplyOptions {
                yes: false,
                prompter: RecordingPrompter::new(ApplyDecision::Create, Rc::new(Cell::new(0))),
            },
        )
        .expect("apply config");

        assert_eq!(report.created, 0);
        assert_eq!(report.skipped, 1);
        assert!(report.conflicts.is_empty());
        assert_symlink_to(&link, &src);
    }
    #[test]
    fn apply_config_skips_existing_relative_symlink_resolving_to_configured_absolute_src() {
        let temp = TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.json");
        let src = source_file(&temp, "source.txt");
        let link_dir = temp.path().join("links");
        fs::create_dir_all(&link_dir).expect("create link parent");
        let link = link_dir.join("link.txt");
        unix_fs::symlink(Path::new("../source.txt"), &link).expect("create relative symlink");
        write_config(&config_path, vec![LinkEntry::new(&link, &src)]);

        let report = apply_config(
            &config_path,
            ApplyOptions {
                yes: false,
                prompter: RecordingPrompter::new(ApplyDecision::Create, Rc::new(Cell::new(0))),
            },
        )
        .expect("apply config");

        assert_eq!(report.created, 0);
        assert_eq!(report.skipped, 1);
        assert!(report.conflicts.is_empty());
        let target = fs::read_link(&link).expect("read symlink");
        assert_eq!(target, Path::new("../source.txt"));
        assert_eq!(
            paths::resolve_symlink_target_lexical(&link, &target).expect("resolve symlink"),
            paths::absolute_lexical(&src).expect("absolute source")
        );
    }
    #[test]
    fn apply_config_reports_conflict_for_symlink_pointing_to_wrong_source() {
        let temp = TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.json");
        let src = source_file(&temp, "source.txt");
        let other_src = source_file(&temp, "other-source.txt");
        let link = temp.path().join("link.txt");
        unix_fs::symlink(&other_src, &link).expect("create conflicting symlink");
        write_config(&config_path, vec![LinkEntry::new(&link, &src)]);

        let report = apply_config(
            &config_path,
            ApplyOptions {
                yes: false,
                prompter: RecordingPrompter::new(ApplyDecision::Create, Rc::new(Cell::new(0))),
            },
        )
        .expect("apply config");

        assert_eq!(report.created, 0);
        assert_eq!(report.skipped, 0);
        assert_eq!(
            report.conflicts,
            vec![ApplyConflict {
                link: link.clone(),
                src: src.clone(),
            }]
        );
        assert_symlink_to(&link, &other_src);
    }

    #[test]
    fn apply_config_skips_missing_link_parent_without_creating_it() {
        let temp = TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.json");
        let src = source_file(&temp, "source.txt");
        let missing_parent = temp.path().join("missing-parent");
        let link = missing_parent.join("link.txt");
        write_config(&config_path, vec![LinkEntry::new(&link, &src)]);

        let report = apply_config(
            &config_path,
            ApplyOptions {
                yes: false,
                prompter: RecordingPrompter::new(ApplyDecision::Create, Rc::new(Cell::new(0))),
            },
        )
        .expect("apply config");

        assert_eq!(report.created, 0);
        assert_eq!(report.skipped, 1);
        assert!(report.conflicts.is_empty());
        assert!(!missing_parent.exists());
        assert!(!link.exists());
    }

    #[test]
    fn apply_config_creates_missing_link_after_interactive_create_decision() {
        let temp = TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.json");
        let src = source_file(&temp, "source.txt");
        let link = temp.path().join("link.txt");
        write_config(&config_path, vec![LinkEntry::new(&link, &src)]);
        let calls = Rc::new(Cell::new(0));
        let prompter = RecordingPrompter::new(ApplyDecision::Create, Rc::clone(&calls));

        let report = apply_config(
            &config_path,
            ApplyOptions {
                yes: false,
                prompter,
            },
        )
        .expect("apply config");

        assert_eq!(report.created, 1);
        assert_eq!(report.skipped, 0);
        assert!(report.conflicts.is_empty());
        assert_symlink_to(&link, &src);
        assert_eq!(calls.get(), 1);
    }
    #[test]
    fn apply_config_skips_missing_link_after_interactive_skip_decision() {
        let temp = TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.json");
        let src = source_file(&temp, "source.txt");
        let link = temp.path().join("link.txt");
        write_config(&config_path, vec![LinkEntry::new(&link, &src)]);
        let calls = Rc::new(Cell::new(0));
        let prompter = RecordingPrompter::new(ApplyDecision::Skip, Rc::clone(&calls));

        let report = apply_config(
            &config_path,
            ApplyOptions {
                yes: false,
                prompter,
            },
        )
        .expect("apply config");

        assert_eq!(report.created, 0);
        assert_eq!(report.skipped, 1);
        assert!(report.conflicts.is_empty());
        assert!(!link.exists());
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn apply_config_yes_creates_missing_link_without_prompting() {
        let temp = TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.json");
        let src = source_file(&temp, "source.txt");
        let link = temp.path().join("link.txt");
        write_config(&config_path, vec![LinkEntry::new(&link, &src)]);
        let calls = Rc::new(Cell::new(0));
        let prompter = RecordingPrompter::new(ApplyDecision::Skip, Rc::clone(&calls));

        let report = apply_config(
            &config_path,
            ApplyOptions {
                yes: true,
                prompter,
            },
        )
        .expect("apply config");

        assert_eq!(report.created, 1);
        assert_eq!(report.skipped, 0);
        assert!(report.conflicts.is_empty());
        assert_symlink_to(&link, &src);
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn apply_config_reports_regular_file_conflict_without_overwriting() {
        let temp = TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.json");
        let src = source_file(&temp, "source.txt");
        let link = temp.path().join("link.txt");
        fs::write(&link, "existing").expect("write conflicting regular file");
        write_config(&config_path, vec![LinkEntry::new(&link, &src)]);

        let report = apply_config(
            &config_path,
            ApplyOptions {
                yes: true,
                prompter: RecordingPrompter::new(ApplyDecision::Create, Rc::new(Cell::new(0))),
            },
        )
        .expect("apply config");

        assert_eq!(report.created, 0);
        assert_eq!(report.skipped, 0);
        assert_eq!(
            report.conflicts,
            vec![ApplyConflict {
                link: link.clone(),
                src: src.clone(),
            }]
        );
        assert_eq!(
            fs::read_to_string(&link).expect("read conflict"),
            "existing"
        );
        assert!(
            !fs::symlink_metadata(&link)
                .expect("link metadata")
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn apply_config_rejects_invalid_config_without_creating_links() {
        let temp = TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.json");
        let src = source_file(&temp, "source.txt");
        let link = temp.path().join("link.txt");
        write_config_with_version(
            &config_path,
            CURRENT_VERSION + 1,
            vec![LinkEntry::new(&link, &src)],
        );

        let result = apply_config(
            &config_path,
            ApplyOptions {
                yes: true,
                prompter: RecordingPrompter::new(ApplyDecision::Create, Rc::new(Cell::new(0))),
            },
        );

        assert!(matches!(
            result,
            Err(ApplyError::Config(ConfigError::UnsupportedVersion { version }))
                if version == CURRENT_VERSION + 1
        ));
        assert!(!link.exists());
    }
}
// LCOV_EXCL_STOP
