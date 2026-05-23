use crate::{
    config::{ConfigError, ConfigFile, LinkEntry, MergeStatus},
    paths,
};
use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentDecision {
    Create,
    Skip,
}

pub trait ParentPrompter {
    fn decide_create_parent(&mut self, parent: &Path) -> Result<ParentDecision, LinkError>;
}
impl<T: ParentPrompter + ?Sized> ParentPrompter for &mut T {
    fn decide_create_parent(&mut self, parent: &Path) -> Result<ParentDecision, LinkError> {
        (**self).decide_create_parent(parent)
    }
}

#[derive(Debug)]
pub struct LinkOptions<P> {
    pub yes: bool,
    pub prompter: P,
}

#[derive(Debug, PartialEq, Eq)]
pub struct LinkReport {
    pub created_link: bool,
    pub created_parent: bool,
    pub registered: bool,
    pub duplicate: bool,
}

#[derive(Debug)]
pub enum LinkError {
    MissingSource {
        path: PathBuf,
    },
    ParentDeclined {
        parent: PathBuf,
    },
    ParentMissing {
        parent: PathBuf,
    },
    FilesystemConflict {
        path: PathBuf,
    },
    ConfigConflict {
        link: PathBuf,
        existing_src: PathBuf,
        new_src: PathBuf,
    },
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Config {
        source: crate::config::ConfigError,
    },
}

pub fn link_and_register<P: ParentPrompter>(
    src: &Path,
    link: &Path,
    config_path: &Path,
    mut options: LinkOptions<P>,
) -> Result<LinkReport, LinkError> {
    let src = paths::absolute_lexical(src).map_err(|source| LinkError::Io {
        path: src.to_path_buf(),
        source,
    })?;
    let link = paths::absolute_lexical(link).map_err(|source| LinkError::Io {
        path: link.to_path_buf(),
        source,
    })?;

    if !src.exists() {
        return Err(LinkError::MissingSource { path: src });
    }

    let mut config =
        ConfigFile::load_or_default(config_path).map_err(|source| LinkError::Config { source })?;
    paths::normalize_config_entries(&mut config).map_err(|source| LinkError::Io {
        path: config_path.to_path_buf(),
        source,
    })?;
    let merge_status = config
        .merge_entry(LinkEntry::new(link.clone(), src.clone()))
        .map_err(LinkError::from_config_error)?;

    let registered = merge_status == MergeStatus::Added;
    let duplicate = merge_status == MergeStatus::Duplicate;

    let mut created_parent = false;
    if let Some(parent) = link
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .filter(|parent| !parent.exists())
    {
        if options.yes {
            create_parent(parent)?;
            created_parent = true;
        } else {
            match options.prompter.decide_create_parent(parent)? {
                ParentDecision::Create => {
                    create_parent(parent)?;
                    created_parent = true;
                }
                ParentDecision::Skip => {
                    return Err(LinkError::ParentDeclined {
                        parent: parent.to_path_buf(),
                    });
                }
            }
        }
    }

    let created_link = match fs::symlink_metadata(&link) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let existing_target = fs::read_link(&link).map_err(|err| LinkError::Io {
                path: link.to_path_buf(),
                source: err,
            })?;

            let existing_target = paths::resolve_symlink_target_lexical(&link, &existing_target)
                .map_err(|err| LinkError::Io {
                    path: link.to_path_buf(),
                    source: err,
                })?;
            if existing_target != src {
                return Err(LinkError::FilesystemConflict {
                    path: link.to_path_buf(),
                });
            }

            false
        }
        Ok(_) => {
            return Err(LinkError::FilesystemConflict {
                path: link.to_path_buf(),
            });
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            unix_fs::symlink(&src, &link).map_err(|err| LinkError::Io {
                path: link.to_path_buf(),
                source: err,
            })?;
            true
        }
        Err(err) => {
            return Err(LinkError::Io {
                path: link.to_path_buf(),
                source: err,
            });
        }
    };

    if let Err(source) = config.save(config_path) {
        if created_link {
            let _ = remove_created_symlink(&link, &src);
        }

        return Err(LinkError::Config { source });
    }

    Ok(LinkReport {
        created_link,
        created_parent,
        registered,
        duplicate,
    })
}

fn create_parent(parent: &Path) -> Result<(), LinkError> {
    fs::create_dir_all(parent).map_err(|err| LinkError::Io {
        path: parent.to_path_buf(),
        source: err,
    })
}

fn remove_created_symlink(link: &Path, src: &Path) -> Result<(), LinkError> {
    let metadata = match fs::symlink_metadata(link) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(LinkError::Io {
                path: link.to_path_buf(),
                source,
            });
        }
    };

    if !metadata.file_type().is_symlink() {
        return Ok(());
    }

    let target = fs::read_link(link).map_err(|source| LinkError::Io {
        path: link.to_path_buf(),
        source,
    })?;
    let target =
        paths::resolve_symlink_target_lexical(link, &target).map_err(|source| LinkError::Io {
            path: link.to_path_buf(),
            source,
        })?;

    if target == src {
        fs::remove_file(link).map_err(|source| LinkError::Io {
            path: link.to_path_buf(),
            source,
        })?;
    }

    Ok(())
}

impl LinkError {
    fn from_config_error(error: ConfigError) -> Self {
        match error {
            ConfigError::Conflict {
                link,
                existing_src,
                new_src,
            } => LinkError::ConfigConflict {
                link,
                existing_src,
                new_src,
            },
            other => LinkError::Config { source: other },
        }
    }
}

impl fmt::Display for LinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinkError::MissingSource { path } => {
                write!(formatter, "source does not exist: {path:?}")
            }
            LinkError::ParentDeclined { parent } => {
                write!(formatter, "parent directory creation declined: {parent:?}")
            }
            LinkError::ParentMissing { parent } => {
                write!(formatter, "parent directory is missing: {parent:?}")
            }
            LinkError::FilesystemConflict { path } => {
                write!(
                    formatter,
                    "link path already exists and will not be overwritten: {path:?}"
                )
            }
            LinkError::ConfigConflict {
                link,
                existing_src,
                new_src,
            } => write!(
                formatter,
                "config entry for {link:?} already points to {existing_src:?}, not {new_src:?}"
            ),
            LinkError::Io { path, source } => {
                write!(formatter, "I/O error at {path:?}: {source}")
            }
            LinkError::Config { source } => write!(formatter, "config error: {source}"),
        }
    }
}

impl std::error::Error for LinkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LinkError::Io { source, .. } => Some(source),
            LinkError::Config { source } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CURRENT_VERSION, ConfigFile, LinkEntry};
    use std::collections::VecDeque;
    use std::env;
    use std::fs;
    use std::os::unix::fs::{self as unix_fs, MetadataExt, PermissionsExt};

    #[derive(Debug, Default)]
    struct RecordingPrompter {
        decisions: VecDeque<ParentDecision>,
        calls: Vec<PathBuf>,
    }

    impl RecordingPrompter {
        fn with_decision(decision: ParentDecision) -> Self {
            Self {
                decisions: VecDeque::from([decision]),
                calls: Vec::new(),
            }
        }
    }

    impl ParentPrompter for RecordingPrompter {
        fn decide_create_parent(&mut self, parent: &Path) -> Result<ParentDecision, LinkError> {
            self.calls.push(parent.to_path_buf());
            self.decisions
                .pop_front()
                .ok_or_else(|| LinkError::ParentMissing {
                    parent: parent.to_path_buf(),
                })
        }
    }

    struct PanicPrompter;

    impl ParentPrompter for PanicPrompter {
        fn decide_create_parent(&mut self, parent: &Path) -> Result<ParentDecision, LinkError> {
            panic!("prompter must not be called for {parent:?}")
        }
    }
    struct CurrentDirGuard {
        original: PathBuf,
    }

    impl CurrentDirGuard {
        fn change_to(path: &Path) -> Self {
            let original = env::current_dir().expect("read current directory");
            env::set_current_dir(path).expect("change current directory");
            Self { original }
        }
    }

    impl Drop for CurrentDirGuard {
        fn drop(&mut self) {
            env::set_current_dir(&self.original).expect("restore current directory");
        }
    }

    fn temp_root() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let root = dir
            .path()
            .canonicalize()
            .expect("canonicalize temporary directory");
        (dir, root)
    }

    fn write_source(root: &Path, relative: &str) -> PathBuf {
        let src = root.join(relative);
        if let Some(parent) = src.parent() {
            fs::create_dir_all(parent).expect("create source parent");
        }
        fs::write(&src, "source contents\n").expect("write source file");
        src
    }

    fn assert_symlink_to(link: &Path, src: &Path) {
        let metadata = fs::symlink_metadata(link).expect("read link metadata");
        assert!(metadata.file_type().is_symlink());
        assert_eq!(fs::read_link(link).expect("read symlink target"), src);
    }

    fn load_config(path: &Path) -> ConfigFile {
        ConfigFile::load(path).expect("load config")
    }

    fn save_config(path: &Path, links: Vec<LinkEntry>) {
        let mut config = ConfigFile {
            version: CURRENT_VERSION,
            links,
        };
        config.save(path).expect("save config");
    }

    #[test]
    fn creates_symlink_and_registers_absolute_link_and_src_in_config() {
        let (_dir, root) = temp_root();
        let src = write_source(&root, "src/app.toml");
        let link = root.join("links/app.toml");
        fs::create_dir_all(link.parent().expect("link parent")).expect("create link parent");
        let config_path = root.join("symbolic.json");

        let report = link_and_register(
            &src,
            &link,
            &config_path,
            LinkOptions {
                yes: false,
                prompter: RecordingPrompter::default(),
            },
        )
        .expect("link and register");

        assert_eq!(
            report,
            LinkReport {
                created_link: true,
                created_parent: false,
                registered: true,
                duplicate: false,
            }
        );
        assert_symlink_to(&link, &src);
        assert_eq!(
            load_config(&config_path).links,
            vec![LinkEntry::new(link, src)]
        );
    }
    #[test]
    fn creates_symlink_from_relative_paths_and_registers_normalized_absolute_paths() {
        let (_dir, root) = temp_root();
        let src = write_source(&root, "src/app.toml");
        let link = root.join("links/app.toml");
        fs::create_dir_all(link.parent().expect("link parent")).expect("create link parent");
        let config_path = root.join("symbolic.json");
        let _cwd = CurrentDirGuard::change_to(&root);

        let report = link_and_register(
            Path::new("src/app.toml"),
            Path::new("links/app.toml"),
            &config_path,
            LinkOptions {
                yes: false,
                prompter: RecordingPrompter::default(),
            },
        )
        .expect("link and register relative paths");

        assert!(report.created_link);
        assert_eq!(link.canonicalize().expect("resolve created symlink"), src);
        assert_eq!(
            load_config(&config_path).links,
            vec![LinkEntry::new(link, src)]
        );
    }

    #[test]
    fn rolls_back_new_symlink_when_config_save_fails() {
        let (_dir, root) = temp_root();
        let src = write_source(&root, "src/app.toml");
        let link = root.join("links/app.toml");
        fs::create_dir_all(link.parent().expect("link parent")).expect("create link parent");
        let config_path = root.join("symbolic.json");
        save_config(&config_path, Vec::new());
        let mut permissions = fs::metadata(&config_path)
            .expect("config metadata")
            .permissions();
        permissions.set_mode(0o444);
        fs::set_permissions(&config_path, permissions).expect("make config read-only");

        let err = link_and_register(
            &src,
            &link,
            &config_path,
            LinkOptions {
                yes: false,
                prompter: RecordingPrompter::default(),
            },
        )
        .expect_err("config save should fail");

        assert!(matches!(err, LinkError::Config { .. }));
        assert!(!link.exists());
        assert!(fs::symlink_metadata(&link).is_err());
    }

    #[test]
    fn missing_src_fails_without_creating_link_or_config() {
        let (_dir, root) = temp_root();
        let src = root.join("missing/source.toml");
        let link = root.join("links/app.toml");
        fs::create_dir_all(link.parent().expect("link parent")).expect("create link parent");
        let config_path = root.join("symbolic.json");

        let err = link_and_register(
            &src,
            &link,
            &config_path,
            LinkOptions {
                yes: false,
                prompter: RecordingPrompter::default(),
            },
        )
        .expect_err("missing source must fail");

        assert!(matches!(err, LinkError::MissingSource { path } if path == src));
        assert!(!link.exists());
        assert!(!config_path.exists());
    }

    #[test]
    fn interactive_create_decision_creates_missing_parent_and_symlink() {
        let (_dir, root) = temp_root();
        let src = write_source(&root, "src/app.toml");
        let link = root.join("missing/parent/app.toml");
        let parent = link.parent().expect("link parent").to_path_buf();
        let config_path = root.join("symbolic.json");

        let mut prompter = RecordingPrompter::with_decision(ParentDecision::Create);
        let report = link_and_register(
            &src,
            &link,
            &config_path,
            LinkOptions {
                yes: false,
                prompter: &mut prompter,
            },
        )
        .expect("link and register");

        assert_eq!(prompter.calls, vec![parent.clone()]);
        assert!(parent.is_dir());
        assert_symlink_to(&link, &src);
        assert!(report.created_parent);
        assert!(report.created_link);
    }

    #[test]
    fn yes_creates_missing_parent_without_calling_prompter() {
        let (_dir, root) = temp_root();
        let src = write_source(&root, "src/app.toml");
        let link = root.join("missing/parent/app.toml");
        let parent = link.parent().expect("link parent").to_path_buf();
        let config_path = root.join("symbolic.json");

        let report = link_and_register(
            &src,
            &link,
            &config_path,
            LinkOptions {
                yes: true,
                prompter: PanicPrompter,
            },
        )
        .expect("link and register");

        assert!(parent.is_dir());
        assert_symlink_to(&link, &src);
        assert!(report.created_parent);
        assert!(report.created_link);
    }

    #[test]
    fn existing_correct_symlink_is_left_unchanged_and_registration_reports_duplicate_when_present()
    {
        let (_dir, root) = temp_root();
        let src = write_source(&root, "src/app.toml");
        let link = root.join("links/app.toml");
        fs::create_dir_all(link.parent().expect("link parent")).expect("create link parent");
        unix_fs::symlink(&src, &link).expect("create existing symlink");
        let original_metadata =
            fs::symlink_metadata(&link).expect("read original symlink metadata");
        let config_path = root.join("symbolic.json");

        let first_report = link_and_register(
            &src,
            &link,
            &config_path,
            LinkOptions {
                yes: false,
                prompter: RecordingPrompter::default(),
            },
        )
        .expect("register existing symlink");

        assert_eq!(
            first_report,
            LinkReport {
                created_link: false,
                created_parent: false,
                registered: true,
                duplicate: false,
            }
        );
        assert_symlink_to(&link, &src);
        let current_metadata = fs::symlink_metadata(&link).expect("read symlink metadata");
        assert_eq!(current_metadata.dev(), original_metadata.dev());
        assert_eq!(current_metadata.ino(), original_metadata.ino());
        assert_eq!(
            load_config(&config_path).links,
            vec![LinkEntry::new(link.clone(), src.clone())]
        );

        let duplicate_report = link_and_register(
            &src,
            &link,
            &config_path,
            LinkOptions {
                yes: false,
                prompter: RecordingPrompter::default(),
            },
        )
        .expect("register duplicate existing symlink");

        assert_eq!(
            duplicate_report,
            LinkReport {
                created_link: false,
                created_parent: false,
                registered: false,
                duplicate: true,
            }
        );
        assert_symlink_to(&link, &src);
    }

    #[test]
    fn existing_regular_file_at_link_is_conflict_and_is_not_overwritten() {
        let (_dir, root) = temp_root();
        let src = write_source(&root, "src/app.toml");
        let link = root.join("links/app.toml");
        fs::create_dir_all(link.parent().expect("link parent")).expect("create link parent");
        fs::write(&link, "existing file\n").expect("write existing file");
        let config_path = root.join("symbolic.json");

        let err = link_and_register(
            &src,
            &link,
            &config_path,
            LinkOptions {
                yes: false,
                prompter: RecordingPrompter::default(),
            },
        )
        .expect_err("regular file at link must conflict");

        assert!(matches!(err, LinkError::FilesystemConflict { path } if path == link));
        assert_eq!(
            fs::read_to_string(&link).expect("read existing file"),
            "existing file\n"
        );
        assert!(!config_path.exists());
    }

    #[test]
    fn config_entry_with_same_link_and_different_src_is_conflict_without_mutating_filesystem() {
        let (_dir, root) = temp_root();
        let src = write_source(&root, "src/app.toml");
        let other_src = write_source(&root, "src/other.toml");
        let link = root.join("links/app.toml");
        fs::create_dir_all(link.parent().expect("link parent")).expect("create link parent");
        let config_path = root.join("symbolic.json");
        save_config(
            &config_path,
            vec![LinkEntry::new(link.clone(), other_src.clone())],
        );

        let err = link_and_register(
            &src,
            &link,
            &config_path,
            LinkOptions {
                yes: false,
                prompter: RecordingPrompter::default(),
            },
        )
        .expect_err("conflicting config entry must fail");

        assert!(
            matches!(err, LinkError::ConfigConflict { link: error_link, existing_src, new_src }
                if error_link == link && existing_src == other_src && new_src == src)
        );
        assert!(!link.exists());
        assert_eq!(
            load_config(&config_path).links,
            vec![LinkEntry::new(link, other_src)]
        );
    }
}
