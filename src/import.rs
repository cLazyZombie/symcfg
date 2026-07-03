use std::{
    fs,
    io::{self, ErrorKind},
    os::unix::fs::{self as unix_fs, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::{
    config::{ConfigError, ConfigFile, LinkEntry, MergeStatus},
    paths,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportDecision {
    Import,
    Skip,
}

pub trait ImportPrompter {
    fn decide_import(&mut self, link: &Path, src: &Path) -> Result<ImportDecision, ImportError>;
}

#[derive(Debug)]
pub struct ImportOptions<P> {
    pub yes: bool,
    pub prompter: P,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ImportReport {
    pub status: ImportItemStatus,
    pub link: PathBuf,
    pub src: PathBuf,
    pub created_parent: bool,
    pub registered: bool,
    pub duplicate: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportItemStatus {
    Imported,
    Skipped(ImportSkipReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportSkipReason {
    Declined,
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("link path does not exist and cannot be imported: {path:?}")]
    MissingLink { path: PathBuf },

    #[error("link path is already a symlink; use search or link instead: {path:?}")]
    LinkAlreadySymlink { path: PathBuf },

    #[error("link path is not a regular file and will not be imported: {path:?}")]
    LinkNotRegularFile { path: PathBuf },

    #[error("source path already exists and will not be overwritten: {path:?}")]
    SourceExists { path: PathBuf },

    #[error("source path must not be the config file: {path:?}")]
    SourceIsConfig { path: PathBuf },

    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error("I/O error at {path:?}: {source}")]
    Io { path: PathBuf, source: io::Error },

    #[error("prompt failed: {message}")]
    Prompt { message: String },
}

pub fn import_and_register<P: ImportPrompter>(
    link: &Path,
    src: &Path,
    config_path: &Path,
    options: ImportOptions<P>,
) -> Result<ImportReport, ImportError> {
    let link = paths::absolute_lexical(link).map_err(
        // LCOV_EXCL_START
        |source| ImportError::Io {
            path: link.to_path_buf(),
            source,
        },
    )?;
    // LCOV_EXCL_STOP
    let src = paths::absolute_lexical(src).map_err(
        // LCOV_EXCL_START
        |source| ImportError::Io {
            path: src.to_path_buf(),
            source,
        },
    )?;
    // LCOV_EXCL_STOP
    let config_path = paths::absolute_lexical(config_path).map_err(
        // LCOV_EXCL_START
        |source| ImportError::Io {
            path: config_path.to_path_buf(),
            source,
        },
    )?;
    // LCOV_EXCL_STOP

    let source_is_config = paths_may_refer_to_same_file(&src, &config_path).map_err(
        // LCOV_EXCL_START
        |source| ImportError::Io {
            path: src.clone(),
            source,
        },
    )?;
    // LCOV_EXCL_STOP
    if source_is_config {
        return Err(ImportError::SourceIsConfig { path: src });
    }

    let link_metadata = match fs::symlink_metadata(&link) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            return Err(ImportError::MissingLink { path: link });
        }
        // LCOV_EXCL_START
        Err(source) => {
            return Err(ImportError::Io { path: link, source });
        } // LCOV_EXCL_STOP
    };

    if link_metadata.file_type().is_symlink() {
        return Err(ImportError::LinkAlreadySymlink { path: link });
    }

    if !link_metadata.file_type().is_file() {
        return Err(ImportError::LinkNotRegularFile { path: link });
    }

    match fs::symlink_metadata(&src) {
        Ok(_) => return Err(ImportError::SourceExists { path: src }),
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        // LCOV_EXCL_START
        Err(source) => {
            return Err(ImportError::Io { path: src, source });
        } // LCOV_EXCL_STOP
    }

    let ImportOptions { yes, mut prompter } = options;
    let decision = if yes {
        ImportDecision::Import
    } else {
        prompter.decide_import(&link, &src)?
    };

    if decision == ImportDecision::Skip {
        return Ok(ImportReport {
            status: ImportItemStatus::Skipped(ImportSkipReason::Declined),
            link,
            src,
            created_parent: false,
            registered: false,
            duplicate: false,
        });
    }

    let mut config = ConfigFile::load_or_default(&config_path).map_err(ImportError::Config)?;
    paths::normalize_config_entries(&mut config).map_err(
        // LCOV_EXCL_START
        |source| ImportError::Io {
            path: config_path.to_path_buf(),
            source,
        },
    )?;
    // LCOV_EXCL_STOP

    let merge_status = config.merge_entry(LinkEntry::new(link.clone(), src.clone()))?;
    let registered = merge_status == MergeStatus::Added;
    let duplicate = merge_status == MergeStatus::Duplicate;

    let mut created_parent = false;
    if let Some(parent) = src.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        fs::create_dir_all(parent).map_err(
            // LCOV_EXCL_START
            |source| ImportError::Io {
                path: parent.to_path_buf(),
                source,
            },
        )?;
        // LCOV_EXCL_STOP
        created_parent = true;
    }

    let source_mode = link_metadata.permissions().mode() & 0o7777;
    copy_file_create_new(&link, &src, source_mode)?;
    if let Err(source) = fs::remove_file(&link) {
        let _ = rollback_copied_source(&link, &src);
        return Err(ImportError::Io {
            path: link.clone(),
            source,
        });
    }

    // LCOV_EXCL_START
    if let Err(source) = unix_fs::symlink(&src, &link) {
        let _ = rollback_copied_source(&link, &src);
        return Err(ImportError::Io {
            path: link.clone(),
            source,
        });
    }
    // LCOV_EXCL_STOP

    if let Err(source) = config.save(&config_path) {
        // LCOV_EXCL_START
        let _ = rollback_copied_source(&link, &src);
        return Err(ImportError::Config(source));
        // LCOV_EXCL_STOP
    }

    Ok(ImportReport {
        status: ImportItemStatus::Imported,
        link,
        src,
        created_parent,
        registered,
        duplicate,
    })
}

fn paths_may_refer_to_same_file(left: &Path, right: &Path) -> io::Result<bool> {
    if left == right {
        return Ok(true);
    }

    Ok(resolve_existing_ancestor(left)? == resolve_existing_ancestor(right)?)
}

fn resolve_existing_ancestor(path: &Path) -> io::Result<PathBuf> {
    let mut missing_components = Vec::new();
    let mut candidate = path;

    loop {
        match fs::canonicalize(candidate) {
            Ok(mut resolved) => {
                for component in missing_components.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(err) if err.kind() == ErrorKind::NotFound => {
                let name = candidate
                    .file_name()
                    .expect("absolute import paths have an existing root ancestor");
                missing_components.push(PathBuf::from(name));
                candidate = candidate
                    .parent()
                    .expect("absolute import paths have parents before root");
            }
            // LCOV_EXCL_START
            Err(err) => return Err(err),
            // LCOV_EXCL_STOP
        }
    }
}

fn copy_file_create_new(link: &Path, src: &Path, source_mode: u32) -> Result<(), ImportError> {
    let mut source_file = fs::File::open(link).map_err(
        // LCOV_EXCL_START
        |source| ImportError::Io {
            path: link.to_path_buf(),
            source,
        },
    )?;
    // LCOV_EXCL_STOP
    let mut destination_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(source_mode)
        .open(src)
        .map_err(|source| match source.kind() {
            ErrorKind::AlreadyExists => ImportError::SourceExists {
                path: src.to_path_buf(),
            },
            // LCOV_EXCL_START
            _ => ImportError::Io {
                path: src.to_path_buf(),
                source,
            },
            // LCOV_EXCL_STOP
        })?;

    io::copy(&mut source_file, &mut destination_file).map_err(
        // LCOV_EXCL_START
        |source| {
            let _ = fs::remove_file(src);
            ImportError::Io {
                path: src.to_path_buf(),
                source,
            }
        },
    )?;
    // LCOV_EXCL_STOP

    fs::set_permissions(src, fs::Permissions::from_mode(source_mode)).map_err(
        // LCOV_EXCL_START
        |source| {
            let _ = fs::remove_file(src);
            ImportError::Io {
                path: src.to_path_buf(),
                source,
            }
        },
    )?;
    // LCOV_EXCL_STOP

    Ok(())
}

// LCOV_EXCL_START
fn rollback_copied_source(link: &Path, src: &Path) -> Result<(), ImportError> {
    let mut link_exists = false;

    match fs::symlink_metadata(link) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let target = fs::read_link(link).map_err(|source| ImportError::Io {
                path: link.to_path_buf(),
                source,
            })?;
            let target =
                paths::resolve_symlink_target_lexical(link, &target).map_err(|source| {
                    ImportError::Io {
                        path: link.to_path_buf(),
                        source,
                    }
                })?;

            if paths::paths_equivalent(&target, src) {
                fs::remove_file(link).map_err(|source| ImportError::Io {
                    path: link.to_path_buf(),
                    source,
                })?;
            } else {
                link_exists = true;
            }
        }
        Ok(_) => link_exists = true,
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(source) => {
            return Err(ImportError::Io {
                path: link.to_path_buf(),
                source,
            });
        }
    }

    if !src.exists() {
        return Ok(());
    }

    if link_exists {
        fs::remove_file(src).map_err(|source| ImportError::Io {
            path: src.to_path_buf(),
            source,
        })?;
    } else {
        fs::rename(src, link).map_err(|source| ImportError::Io {
            path: link.to_path_buf(),
            source,
        })?;
    }

    Ok(())
}
// LCOV_EXCL_STOP

// LCOV_EXCL_START
#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    struct SkipPrompter;

    impl ImportPrompter for SkipPrompter {
        fn decide_import(
            &mut self,
            _link: &Path,
            _src: &Path,
        ) -> Result<ImportDecision, ImportError> {
            Ok(ImportDecision::Skip)
        }
    }

    #[test]
    fn declined_import_reports_no_registration_side_effect() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let link = dir.path().join("app/config.toml");
        let src = dir.path().join("sources/config.toml");
        let config = dir.path().join("symbolic.json");
        fs::create_dir_all(link.parent().expect("link parent")).expect("create link parent");
        fs::write(&link, "from app\n").expect("write link file");

        let report = import_and_register(
            &link,
            &src,
            &config,
            ImportOptions {
                yes: false,
                prompter: SkipPrompter,
            },
        )
        .expect("declined import should return a skipped report");

        assert_eq!(
            report,
            ImportReport {
                status: ImportItemStatus::Skipped(ImportSkipReason::Declined),
                link,
                src,
                created_parent: false,
                registered: false,
                duplicate: false,
            }
        );
        assert!(!config.exists());
    }

    #[test]
    fn copy_file_create_new_refuses_existing_destination_without_overwriting() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let link = dir.path().join("app/config.toml");
        let src = dir.path().join("sources/config.toml");
        fs::create_dir_all(link.parent().expect("link parent")).expect("create link parent");
        fs::create_dir_all(src.parent().expect("source parent")).expect("create source parent");
        fs::write(&link, "from app\n").expect("write link file");
        fs::write(&src, "from source\n").expect("write existing source file");

        let err = copy_file_create_new(&link, &src, 0o600)
            .expect_err("existing destination should not be overwritten");

        match err {
            ImportError::SourceExists { path } => assert_eq!(path, src),
            other => panic!("expected SourceExists, got {other:?}"),
        }
        assert_eq!(
            fs::read_to_string(&src).expect("read existing source file"),
            "from source\n"
        );
    }
}
// LCOV_EXCL_STOP
