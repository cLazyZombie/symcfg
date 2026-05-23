use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::{
    config::{ConfigError, ConfigFile, LinkEntry},
    paths,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncDeleteDecision {
    DeleteLink,
    KeepLink,
}

pub trait SyncPrompter {
    fn decide_delete_link(&mut self, entry: &LinkEntry) -> Result<SyncDeleteDecision, SyncError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoDeletePolicy {
    DeleteLinks,
    KeepLinks,
}

pub struct SyncOptions<P> {
    pub yes: bool,
    pub auto_delete_policy: Option<AutoDeletePolicy>,
    pub prompter: P,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncReport {
    pub stale: usize,
    pub removed_entries: usize,
    pub deleted_links: usize,
    pub kept_links: usize,
}

pub fn sync_config<P: SyncPrompter>(
    source_root: &Path,
    config_path: &Path,
    options: SyncOptions<P>,
) -> Result<SyncReport, SyncError> {
    if options.yes && options.auto_delete_policy.is_none() {
        return Err(SyncError::MissingAutoDeletePolicy);
    }

    let SyncOptions {
        yes,
        auto_delete_policy,
        mut prompter,
    } = options;

    let mut config = ConfigFile::load(config_path)?;
    paths::normalize_config_entries(&mut config).map_err(|err| io_error(config_path, err))?;
    let source_root =
        paths::absolute_lexical(source_root).map_err(|err| io_error(source_root, err))?;
    let stale_entries: Vec<LinkEntry> = config
        .links
        .iter()
        .filter(|entry| entry.src.starts_with(&source_root) && !entry.src.exists())
        .cloned()
        .collect();

    let mut report = SyncReport {
        stale: stale_entries.len(),
        removed_entries: stale_entries.len(),
        deleted_links: 0,
        kept_links: 0,
    };

    for entry in &stale_entries {
        match delete_decision(yes, auto_delete_policy, &mut prompter, entry)? {
            SyncDeleteDecision::KeepLink => {
                if link_path_exists(&entry.link)? {
                    report.kept_links += 1;
                }
            }
            SyncDeleteDecision::DeleteLink => match delete_matching_symlink(entry)? {
                LinkDeleteStatus::Deleted => report.deleted_links += 1,
                LinkDeleteStatus::Kept => report.kept_links += 1,
                LinkDeleteStatus::Missing => {}
            },
        }
    }

    config.links.retain(|entry| !stale_entries.contains(entry));
    config.save(config_path)?;

    Ok(report)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkDeleteStatus {
    Deleted,
    Kept,
    Missing,
}

fn delete_decision<P: SyncPrompter>(
    yes: bool,
    auto_delete_policy: Option<AutoDeletePolicy>,
    prompter: &mut P,
    entry: &LinkEntry,
) -> Result<SyncDeleteDecision, SyncError> {
    if !yes {
        return prompter.decide_delete_link(entry);
    }

    match auto_delete_policy {
        Some(AutoDeletePolicy::DeleteLinks) => Ok(SyncDeleteDecision::DeleteLink),
        Some(AutoDeletePolicy::KeepLinks) => Ok(SyncDeleteDecision::KeepLink),
        None => Err(SyncError::MissingAutoDeletePolicy),
    }
}

fn link_path_exists(path: &Path) -> Result<bool, SyncError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
        Err(err) => Err(io_error(path, err)),
    }
}

fn delete_matching_symlink(entry: &LinkEntry) -> Result<LinkDeleteStatus, SyncError> {
    let metadata = match fs::symlink_metadata(&entry.link) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(LinkDeleteStatus::Missing),
        Err(err) => return Err(io_error(&entry.link, err)),
    };

    if !metadata.file_type().is_symlink() {
        return Ok(LinkDeleteStatus::Kept);
    }

    let target = fs::read_link(&entry.link).map_err(|err| io_error(&entry.link, err))?;
    let target = paths::resolve_symlink_target_lexical(&entry.link, &target)
        .map_err(|err| io_error(&entry.link, err))?;
    let src = paths::absolute_lexical(&entry.src).map_err(|err| io_error(&entry.src, err))?;
    if target != src {
        return Ok(LinkDeleteStatus::Kept);
    }

    fs::remove_file(&entry.link).map_err(|err| io_error(&entry.link, err))?;
    Ok(LinkDeleteStatus::Deleted)
}

fn io_error(path: &Path, err: std::io::Error) -> SyncError {
    SyncError::Io {
        path: path.to_path_buf(),
        kind: err.kind(),
        message: err.to_string(),
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SyncError {
    #[error("automatic delete policy is required when yes is true")]
    MissingAutoDeletePolicy,

    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error("I/O error at {path:?}: {message}")]
    Io {
        path: PathBuf,
        kind: ErrorKind,
        message: String,
    },

    #[error("prompt failed: {0}")]
    Prompt(String),
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, fs, os::unix::fs::symlink, path::Path, rc::Rc};

    use crate::config::{CURRENT_VERSION, ConfigFile};

    use super::*;

    #[derive(Default)]
    struct NoopPrompter;

    impl SyncPrompter for NoopPrompter {
        fn decide_delete_link(
            &mut self,
            _entry: &LinkEntry,
        ) -> Result<SyncDeleteDecision, SyncError> {
            panic!("prompter must not be called")
        }
    }

    #[derive(Clone)]
    struct RecordingPrompter {
        decisions: Rc<RefCell<Vec<SyncDeleteDecision>>>,
        calls: Rc<RefCell<Vec<LinkEntry>>>,
    }

    impl RecordingPrompter {
        fn new(decisions: Vec<SyncDeleteDecision>) -> Self {
            Self {
                decisions: Rc::new(RefCell::new(decisions)),
                calls: Rc::new(RefCell::new(Vec::new())),
            }
        }

        fn calls(&self) -> Vec<LinkEntry> {
            self.calls.borrow().clone()
        }
    }

    impl SyncPrompter for RecordingPrompter {
        fn decide_delete_link(
            &mut self,
            entry: &LinkEntry,
        ) -> Result<SyncDeleteDecision, SyncError> {
            self.calls.borrow_mut().push(entry.clone());
            let mut decisions = self.decisions.borrow_mut();
            if decisions.is_empty() {
                return Err(SyncError::Prompt(
                    "missing scripted delete decision".to_owned(),
                ));
            }
            Ok(decisions.remove(0))
        }
    }

    fn entry(link: &Path, src: &Path) -> LinkEntry {
        LinkEntry::new(link.to_path_buf(), src.to_path_buf())
    }

    fn write_config(path: &Path, links: Vec<LinkEntry>) {
        let mut config = ConfigFile {
            version: CURRENT_VERSION,
            links,
        };
        config.save(path).expect("save config");
    }

    fn read_links(path: &Path) -> Vec<LinkEntry> {
        ConfigFile::load(path).expect("load config").links
    }

    fn write_source_file(path: &Path) {
        fs::create_dir_all(path.parent().expect("source has parent"))
            .expect("create source parent");
        fs::write(path, "source contents\n").expect("write source file");
    }

    fn assert_symlink_points_to(link: &Path, src: &Path) {
        let metadata = fs::symlink_metadata(link).expect("read symlink metadata");
        assert!(
            metadata.file_type().is_symlink(),
            "path should be a symlink"
        );
        let target = fs::read_link(link).expect("read symlink target");
        assert_eq!(target.as_path(), src);
    }

    #[test]
    fn source_under_source_root_that_exists_remains_in_config() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let source_root = dir.path().join("source-root");
        let src = source_root.join("tool");
        let link = dir.path().join("links/tool");
        let config_path = dir.path().join("symbolic.json");
        let original = entry(&link, &src);
        write_source_file(&src);
        write_config(&config_path, vec![original.clone()]);

        let report = sync_config(
            &source_root,
            &config_path,
            SyncOptions {
                yes: false,
                auto_delete_policy: None,
                prompter: NoopPrompter,
            },
        )
        .expect("sync config");

        assert_eq!(
            report,
            SyncReport {
                stale: 0,
                removed_entries: 0,
                deleted_links: 0,
                kept_links: 0,
            }
        );
        assert_eq!(read_links(&config_path), vec![original]);
    }

    #[test]
    fn missing_source_under_source_root_is_stale_and_removed_from_config() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let source_root = dir.path().join("source-root");
        let stale_src = source_root.join("missing-tool");
        let link = dir.path().join("links/missing-tool");
        let config_path = dir.path().join("symbolic.json");
        write_config(&config_path, vec![entry(&link, &stale_src)]);

        let report = sync_config(
            &source_root,
            &config_path,
            SyncOptions {
                yes: true,
                auto_delete_policy: Some(AutoDeletePolicy::KeepLinks),
                prompter: NoopPrompter,
            },
        )
        .expect("sync config");

        assert_eq!(
            report,
            SyncReport {
                stale: 1,
                removed_entries: 1,
                deleted_links: 0,
                kept_links: 0,
            }
        );
        assert!(read_links(&config_path).is_empty());
    }
    #[test]
    fn stale_source_is_removed_when_source_root_contains_parent_component() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let source_root = dir.path().join("source-root");
        let lexical_source_root = dir.path().join("sibling/../source-root");
        let stale_src = source_root.join("missing-tool");
        let link = dir.path().join("links/missing-tool");
        let config_path = dir.path().join("symbolic.json");
        fs::create_dir_all(dir.path().join("sibling")).expect("create sibling directory");
        fs::create_dir_all(&source_root).expect("create source root");
        write_config(&config_path, vec![entry(&link, &stale_src)]);

        let report = sync_config(
            &lexical_source_root,
            &config_path,
            SyncOptions {
                yes: true,
                auto_delete_policy: Some(AutoDeletePolicy::KeepLinks),
                prompter: NoopPrompter,
            },
        )
        .expect("sync config");

        assert_eq!(
            report,
            SyncReport {
                stale: 1,
                removed_entries: 1,
                deleted_links: 0,
                kept_links: 0,
            }
        );
        assert!(read_links(&config_path).is_empty());
    }

    #[test]
    fn missing_source_outside_source_root_is_not_stale_and_remains_in_config() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let source_root = dir.path().join("source-root");
        let outside_src = dir.path().join("outside/missing-tool");
        let link = dir.path().join("links/missing-tool");
        let config_path = dir.path().join("symbolic.json");
        let original = entry(&link, &outside_src);
        write_config(&config_path, vec![original.clone()]);

        let report = sync_config(
            &source_root,
            &config_path,
            SyncOptions {
                yes: true,
                auto_delete_policy: Some(AutoDeletePolicy::DeleteLinks),
                prompter: NoopPrompter,
            },
        )
        .expect("sync config");

        assert_eq!(
            report,
            SyncReport {
                stale: 0,
                removed_entries: 0,
                deleted_links: 0,
                kept_links: 0,
            }
        );
        assert_eq!(read_links(&config_path), vec![original]);
    }

    #[test]
    fn interactive_keep_link_removes_stale_config_entry_and_leaves_symlink() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let source_root = dir.path().join("source-root");
        let stale_src = source_root.join("missing-tool");
        let link = dir.path().join("links/missing-tool");
        let config_path = dir.path().join("symbolic.json");
        fs::create_dir_all(link.parent().expect("link has parent")).expect("create link parent");
        symlink(&stale_src, &link).expect("create stale symlink");
        let stale_entry = entry(&link, &stale_src);
        write_config(&config_path, vec![stale_entry.clone()]);
        let prompter = RecordingPrompter::new(vec![SyncDeleteDecision::KeepLink]);

        let report = sync_config(
            &source_root,
            &config_path,
            SyncOptions {
                yes: false,
                auto_delete_policy: None,
                prompter: prompter.clone(),
            },
        )
        .expect("sync config");

        assert_eq!(
            report,
            SyncReport {
                stale: 1,
                removed_entries: 1,
                deleted_links: 0,
                kept_links: 1,
            }
        );
        assert!(read_links(&config_path).is_empty());
        assert_symlink_points_to(&link, &stale_src);
        assert_eq!(prompter.calls(), vec![stale_entry]);
    }

    #[test]
    fn yes_keep_links_removes_stale_config_entry_and_keeps_symlink_without_prompting() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let source_root = dir.path().join("source-root");
        let stale_src = source_root.join("missing-tool");
        let link = dir.path().join("links/missing-tool");
        let config_path = dir.path().join("symbolic.json");
        fs::create_dir_all(link.parent().expect("link has parent")).expect("create link parent");
        symlink(&stale_src, &link).expect("create stale symlink");
        write_config(&config_path, vec![entry(&link, &stale_src)]);

        let report = sync_config(
            &source_root,
            &config_path,
            SyncOptions {
                yes: true,
                auto_delete_policy: Some(AutoDeletePolicy::KeepLinks),
                prompter: NoopPrompter,
            },
        )
        .expect("sync config");

        assert_eq!(
            report,
            SyncReport {
                stale: 1,
                removed_entries: 1,
                deleted_links: 0,
                kept_links: 1,
            }
        );
        assert!(read_links(&config_path).is_empty());
        assert_symlink_points_to(&link, &stale_src);
    }

    #[test]
    fn yes_delete_links_removes_stale_config_entry_and_deletes_matching_symlink() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let source_root = dir.path().join("source-root");
        let stale_src = source_root.join("missing-tool");
        let link = dir.path().join("links/missing-tool");
        let config_path = dir.path().join("symbolic.json");
        fs::create_dir_all(link.parent().expect("link has parent")).expect("create link parent");
        symlink(&stale_src, &link).expect("create stale symlink");
        write_config(&config_path, vec![entry(&link, &stale_src)]);

        let report = sync_config(
            &source_root,
            &config_path,
            SyncOptions {
                yes: true,
                auto_delete_policy: Some(AutoDeletePolicy::DeleteLinks),
                prompter: NoopPrompter,
            },
        )
        .expect("sync config");

        assert_eq!(
            report,
            SyncReport {
                stale: 1,
                removed_entries: 1,
                deleted_links: 1,
                kept_links: 0,
            }
        );
        assert!(read_links(&config_path).is_empty());
        assert!(!link.exists());
        assert!(fs::symlink_metadata(&link).is_err());
    }
    #[test]
    fn delete_links_policy_deletes_stale_symlink_with_relative_target_resolving_to_recorded_src() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let source_root = dir.path().join("source-root");
        let stale_src = source_root.join("missing-tool");
        let link = dir.path().join("links/missing-tool");
        let config_path = dir.path().join("symbolic.json");
        fs::create_dir_all(link.parent().expect("link has parent")).expect("create link parent");
        symlink(Path::new("../source-root/missing-tool"), &link)
            .expect("create stale relative symlink");
        write_config(&config_path, vec![entry(&link, &stale_src)]);

        let report = sync_config(
            &source_root,
            &config_path,
            SyncOptions {
                yes: true,
                auto_delete_policy: Some(AutoDeletePolicy::DeleteLinks),
                prompter: NoopPrompter,
            },
        )
        .expect("sync config");

        assert_eq!(
            report,
            SyncReport {
                stale: 1,
                removed_entries: 1,
                deleted_links: 1,
                kept_links: 0,
            }
        );
        assert!(read_links(&config_path).is_empty());
        assert!(fs::symlink_metadata(&link).is_err());
    }

    #[test]
    fn yes_without_delete_policy_fails() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let source_root = dir.path().join("source-root");
        let stale_src = source_root.join("missing-tool");
        let link = dir.path().join("links/missing-tool");
        let config_path = dir.path().join("symbolic.json");
        write_config(&config_path, vec![entry(&link, &stale_src)]);

        let err = sync_config(
            &source_root,
            &config_path,
            SyncOptions {
                yes: true,
                auto_delete_policy: None,
                prompter: NoopPrompter,
            },
        )
        .expect_err("yes without delete policy should fail");

        assert_eq!(err, SyncError::MissingAutoDeletePolicy);
    }

    #[test]
    fn delete_links_policy_never_deletes_regular_files_or_symlinks_to_different_sources() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let source_root = dir.path().join("source-root");
        let stale_regular_src = source_root.join("missing-regular");
        let stale_wrong_symlink_src = source_root.join("missing-wrong-symlink");
        let different_src = source_root.join("different-missing-source");
        let regular_link = dir.path().join("links/regular-file");
        let wrong_symlink = dir.path().join("links/wrong-symlink");
        let config_path = dir.path().join("symbolic.json");
        fs::create_dir_all(regular_link.parent().expect("link has parent"))
            .expect("create link parent");
        fs::write(&regular_link, "user file\n").expect("write regular file at link path");
        symlink(&different_src, &wrong_symlink).expect("create symlink to different source");
        write_config(
            &config_path,
            vec![
                entry(&regular_link, &stale_regular_src),
                entry(&wrong_symlink, &stale_wrong_symlink_src),
            ],
        );

        let report = sync_config(
            &source_root,
            &config_path,
            SyncOptions {
                yes: true,
                auto_delete_policy: Some(AutoDeletePolicy::DeleteLinks),
                prompter: NoopPrompter,
            },
        )
        .expect("sync config");

        assert_eq!(
            report,
            SyncReport {
                stale: 2,
                removed_entries: 2,
                deleted_links: 0,
                kept_links: 2,
            }
        );
        assert!(read_links(&config_path).is_empty());
        assert_eq!(
            fs::read_to_string(&regular_link).expect("read regular file"),
            "user file\n"
        );
        assert_symlink_points_to(&wrong_symlink, &different_src);
        assert!(
            fs::symlink_metadata(&regular_link)
                .expect("regular file metadata")
                .file_type()
                .is_file()
        );
    }
}
