use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    config::{ConfigError, ConfigFile, LinkEntry, MergeStatus},
    paths,
};

use thiserror::Error;
use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchReport {
    pub matched: usize,
    pub added: usize,
    pub duplicates: usize,
    pub conflicts: Vec<SearchConflict>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchConflict {
    pub link: PathBuf,
    pub existing_src: PathBuf,
    pub new_src: PathBuf,
}

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("I/O error at {path:?}: {message}")]
    Io { path: PathBuf, message: String },

    #[error("config error: {0}")]
    Config(#[from] crate::config::ConfigError),

    #[error("walk error at {path:?}: {message}")]
    Walk { path: PathBuf, message: String },
}

pub fn search_and_update_config(
    source_root: &Path,
    link_roots: &[PathBuf],
    config_path: &Path,
) -> Result<SearchReport, SearchError> {
    let source_root = paths::absolute_lexical(source_root).map_err(|err| {
        // LCOV_EXCL_START
        SearchError::Io {
            path: source_root.to_path_buf(),
            message: err.to_string(),
        }
    })?;
    // LCOV_EXCL_STOP

    let mut config = ConfigFile::load_or_default(config_path)?;
    paths::normalize_config_entries(&mut config).map_err(|err| {
        // LCOV_EXCL_START
        SearchError::Io {
            path: config_path.to_path_buf(),
            message: err.to_string(),
        }
    })?;
    // LCOV_EXCL_STOP
    let mut report = SearchReport {
        matched: 0,
        added: 0,
        duplicates: 0,
        conflicts: Vec::new(),
    };

    for link_root in link_roots {
        for entry in WalkDir::new(link_root).follow_links(false) {
            let entry = entry.map_err(|err| {
                // LCOV_EXCL_START
                SearchError::Walk {
                    path: err
                        .path()
                        .map_or_else(|| link_root.to_path_buf(), Path::to_path_buf),
                    message: err.to_string(),
                }
            })?;
            // LCOV_EXCL_STOP

            if !entry.file_type().is_symlink() {
                continue;
            }

            let link = paths::absolute_lexical(entry.path()).map_err(|err| {
                // LCOV_EXCL_START
                SearchError::Io {
                    path: entry.path().to_path_buf(),
                    message: err.to_string(),
                }
            })?;
            // LCOV_EXCL_STOP
            let target = fs::read_link(entry.path()).map_err(|err| {
                // LCOV_EXCL_START
                SearchError::Io {
                    path: entry.path().to_path_buf(),
                    message: err.to_string(),
                }
            })?;
            // LCOV_EXCL_STOP
            let src = paths::resolve_symlink_target_lexical(&link, &target).map_err(|err| {
                // LCOV_EXCL_START
                SearchError::Io {
                    path: target.clone(),
                    message: err.to_string(),
                }
            })?;
            // LCOV_EXCL_STOP

            if !src.starts_with(&source_root) {
                continue;
            }

            report.matched += 1;
            match config.merge_entry(LinkEntry::new(link.clone(), src.clone())) {
                Ok(MergeStatus::Added) => report.added += 1,
                Ok(MergeStatus::Duplicate) => report.duplicates += 1,
                Err(ConfigError::Conflict {
                    existing_src,
                    new_src,
                    ..
                }) => report.conflicts.push(SearchConflict {
                    link,
                    existing_src,
                    new_src,
                }),
                Err(err) => return Err(SearchError::Config(err)), // LCOV_EXCL_LINE
            }
        }
    }

    config.save(config_path)?;
    Ok(report)
}

// LCOV_EXCL_START
#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::symlink, path::Path};

    use crate::config::{CURRENT_VERSION, ConfigFile, LinkEntry};

    use super::*;

    fn touch(path: &Path) {
        fs::write(path, "source file\n").expect("write source file");
    }

    fn load_config(path: &Path) -> ConfigFile {
        ConfigFile::load(path).expect("load config written by search")
    }

    #[test]
    fn finds_symlink_in_link_root_pointing_under_source_root_and_writes_link_and_src_config_entry()
    {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let source_root = dir.path().join("sources");
        let link_root = dir.path().join("links");
        let config_path = dir.path().join("symbolic.json");
        fs::create_dir_all(&source_root).expect("create source root");
        fs::create_dir_all(&link_root).expect("create link root");
        let source = source_root.join("tool");
        let link = link_root.join("tool");
        touch(&source);
        symlink(&source, &link).expect("create symlink under link root");

        let report = search_and_update_config(&source_root, &[link_root], &config_path)
            .expect("search should succeed");

        assert_eq!(report.matched, 1);
        assert_eq!(report.added, 1);
        assert_eq!(report.duplicates, 0);
        assert!(report.conflicts.is_empty());

        let config = load_config(&config_path);
        assert_eq!(config.version, CURRENT_VERSION);
        assert_eq!(config.links, vec![LinkEntry::new(link, source)]);

        let raw_config = fs::read_to_string(&config_path).expect("read config json");
        assert!(raw_config.contains("\"link\""));
        assert!(raw_config.contains("\"src\""));
        assert!(!raw_config.contains("\"target\""));
    }
    #[test]
    fn stores_normalized_absolute_src_for_relative_symlink_target_under_link_root() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let source_root = dir.path().join("sources");
        let link_root = dir.path().join("links");
        let config_path = dir.path().join("symbolic.json");
        fs::create_dir_all(&source_root).expect("create source root");
        fs::create_dir_all(&link_root).expect("create link root");
        let source = source_root.join("tool");
        let link = link_root.join("tool");
        touch(&source);
        symlink(Path::new("../sources/tool"), &link)
            .expect("create relative symlink under link root");

        let report = search_and_update_config(&source_root, &[link_root], &config_path)
            .expect("search should succeed");

        assert_eq!(report.matched, 1);
        assert_eq!(report.added, 1);
        assert_eq!(
            load_config(&config_path).links,
            vec![LinkEntry::new(link, source)]
        );
    }

    #[test]
    fn finds_symlinks_in_nested_directories_under_link_root() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let source_root = dir.path().join("sources");
        let nested_link_dir = dir.path().join("links").join("tools").join("bin");
        let config_path = dir.path().join("symbolic.json");
        fs::create_dir_all(&source_root).expect("create source root");
        fs::create_dir_all(&nested_link_dir).expect("create nested link directory");
        let source = source_root.join("tool");
        let link = nested_link_dir.join("tool");
        touch(&source);
        symlink(&source, &link).expect("create nested symlink");
        let link_root = dir.path().join("links");

        let report = search_and_update_config(&source_root, &[link_root], &config_path)
            .expect("search should succeed");

        assert_eq!(report.matched, 1);
        assert_eq!(report.added, 1);
        let config = load_config(&config_path);
        assert_eq!(config.links, vec![LinkEntry::new(link, source)]);
    }

    #[test]
    fn ignores_symlinks_pointing_outside_source_root() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let source_root = dir.path().join("sources");
        let outside_root = dir.path().join("outside");
        let link_root = dir.path().join("links");
        let config_path = dir.path().join("symbolic.json");
        fs::create_dir_all(&source_root).expect("create source root");
        fs::create_dir_all(&outside_root).expect("create outside root");
        fs::create_dir_all(&link_root).expect("create link root");
        let outside_source = outside_root.join("tool");
        let link = link_root.join("tool");
        let inside_source = source_root.join("kept-tool");
        let inside_link = link_root.join("kept-tool");
        touch(&outside_source);
        symlink(&outside_source, &link).expect("create symlink outside source root");
        touch(&inside_source);
        symlink(&inside_source, &inside_link).expect("create symlink inside source root");

        let report = search_and_update_config(&source_root, &[link_root], &config_path)
            .expect("search should succeed");

        assert_eq!(report.matched, 1);
        assert_eq!(report.added, 1);
        assert_eq!(
            load_config(&config_path).links,
            vec![LinkEntry::new(inside_link, inside_source)]
        );
    }

    #[test]
    fn does_not_follow_symlinked_directories_while_traversing() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let source_root = dir.path().join("sources");
        let link_root = dir.path().join("links");
        let real_nested_dir = dir.path().join("real-nested");
        let config_path = dir.path().join("symbolic.json");
        fs::create_dir_all(&source_root).expect("create source root");
        fs::create_dir_all(&link_root).expect("create link root");
        fs::create_dir_all(&real_nested_dir).expect("create real nested directory");
        let source = source_root.join("tool");
        let nested_link = real_nested_dir.join("tool");
        let symlinked_dir = link_root.join("symlinked-dir");
        let direct_link = link_root.join("direct-tool");
        touch(&source);
        symlink(&source, &nested_link).expect("create symlink inside real nested directory");
        symlink(&source, &direct_link).expect("create direct symlink in link root");
        symlink(&real_nested_dir, &symlinked_dir).expect("create symlinked directory in link root");

        let report = search_and_update_config(&source_root, &[link_root], &config_path)
            .expect("search should succeed");

        assert_eq!(report.matched, 1);
        assert_eq!(report.added, 1);
        assert_eq!(
            load_config(&config_path).links,
            vec![LinkEntry::new(direct_link, source)]
        );
    }

    #[test]
    fn merges_with_existing_config_without_duplicating_already_recorded_link_and_src() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let source_root = dir.path().join("sources");
        let link_root = dir.path().join("links");
        let config_path = dir.path().join("symbolic.json");
        fs::create_dir_all(&source_root).expect("create source root");
        fs::create_dir_all(&link_root).expect("create link root");
        let source = source_root.join("tool");
        let link = link_root.join("tool");
        touch(&source);
        symlink(&source, &link).expect("create symlink under link root");
        let mut existing = ConfigFile {
            version: CURRENT_VERSION,
            links: vec![LinkEntry::new(link.clone(), source.clone())],
        };
        existing.save(&config_path).expect("write existing config");

        let report = search_and_update_config(&source_root, &[link_root], &config_path)
            .expect("search should succeed");

        assert_eq!(report.matched, 1);
        assert_eq!(report.added, 0);
        assert_eq!(report.duplicates, 1);
        assert!(report.conflicts.is_empty());
        assert_eq!(
            load_config(&config_path).links,
            vec![LinkEntry::new(link, source)]
        );
    }

    #[test]
    fn reports_conflict_for_same_link_with_different_src_and_preserves_existing_config_entry() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let source_root = dir.path().join("sources");
        let link_root = dir.path().join("links");
        let config_path = dir.path().join("symbolic.json");
        fs::create_dir_all(&source_root).expect("create source root");
        fs::create_dir_all(&link_root).expect("create link root");
        let existing_source = source_root.join("existing-tool");
        let new_source = source_root.join("new-tool");
        let link = link_root.join("tool");
        touch(&existing_source);
        touch(&new_source);
        symlink(&new_source, &link).expect("create conflicting symlink under link root");
        let mut existing = ConfigFile {
            version: CURRENT_VERSION,
            links: vec![LinkEntry::new(link.clone(), existing_source.clone())],
        };
        existing.save(&config_path).expect("write existing config");

        let report = search_and_update_config(&source_root, &[link_root], &config_path)
            .expect("search should succeed");

        assert_eq!(report.matched, 1);
        assert_eq!(report.added, 0);
        assert_eq!(report.duplicates, 0);
        assert_eq!(
            report.conflicts,
            vec![SearchConflict {
                link: link.clone(),
                existing_src: existing_source.clone(),
                new_src: new_source,
            }]
        );
        assert_eq!(
            load_config(&config_path).links,
            vec![LinkEntry::new(link, existing_source)]
        );
    }
}
// LCOV_EXCL_STOP
