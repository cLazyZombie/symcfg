use std::{
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::{
    config::{ConfigError, ConfigFile},
    paths,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkStatus {
    Linked,
    Missing,
    Conflict,
}

impl LinkStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkStatus::Linked => "linked",
            LinkStatus::Missing => "missing",
            LinkStatus::Conflict => "conflict",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    pub status: LinkStatus,
    pub link: PathBuf,
    pub src: PathBuf,
}

#[derive(Debug, Error)]
pub enum ListError {
    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error("I/O error at {path:?}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

pub fn list_config(config_path: &Path) -> Result<Vec<ListItem>, ListError> {
    let config = ConfigFile::load(config_path)?;
    config
        .links
        .iter()
        .map(|entry| {
            let link = paths::absolute_lexical(&entry.link).map_err(|source| ListError::Io {
                // LCOV_EXCL_START
                path: entry.link.clone(),
                source,
                // LCOV_EXCL_STOP
            })?; // LCOV_EXCL_LINE
            let src = paths::absolute_lexical(&entry.src).map_err(|source| ListError::Io {
                // LCOV_EXCL_START
                path: entry.src.clone(),
                source,
                // LCOV_EXCL_STOP
            })?; // LCOV_EXCL_LINE
            let status = link_status(&link, &src)?;

            Ok(ListItem {
                status,
                link,
                src: entry.src.clone(),
            })
        })
        .collect()
}

fn link_status(link: &Path, src: &Path) -> Result<LinkStatus, ListError> {
    let metadata = match fs::symlink_metadata(link) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(LinkStatus::Missing),
        // LCOV_EXCL_START
        Err(source) => {
            return Err(ListError::Io {
                path: link.to_path_buf(),
                source,
            });
        } // LCOV_EXCL_STOP
    };

    if !metadata.file_type().is_symlink() {
        return Ok(LinkStatus::Conflict);
    }

    let target = fs::read_link(link).map_err(|source| ListError::Io {
        // LCOV_EXCL_START
        path: link.to_path_buf(),
        source,
        // LCOV_EXCL_STOP
    })?; // LCOV_EXCL_LINE
    let target =
        paths::resolve_symlink_target_lexical(link, &target).map_err(|source| ListError::Io {
            // LCOV_EXCL_START
            path: link.to_path_buf(),
            source,
            // LCOV_EXCL_STOP
        })?; // LCOV_EXCL_LINE

    if paths::paths_equivalent(&target, src) {
        Ok(LinkStatus::Linked)
    } else {
        Ok(LinkStatus::Conflict)
    }
}

// LCOV_EXCL_START
#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::symlink, path::Path};

    use crate::config::{CURRENT_VERSION, ConfigFile, LinkEntry};

    use super::*;

    fn write_config(path: &Path, links: Vec<LinkEntry>) {
        let mut config = ConfigFile {
            version: CURRENT_VERSION,
            links,
        };
        config.save(path).expect("save config");
    }

    fn write_source(path: &Path) {
        fs::create_dir_all(path.parent().expect("source parent")).expect("create source parent");
        fs::write(path, "source\n").expect("write source file");
    }

    #[test]
    fn list_reports_linked_missing_and_conflict_entries() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let config_path = dir.path().join("symbolic.json");
        let linked_src = dir.path().join("sources/linked");
        let missing_src = dir.path().join("sources/missing");
        let conflict_src = dir.path().join("sources/conflict");
        let linked_link = dir.path().join("links/linked");
        let missing_link = dir.path().join("links/missing");
        let conflict_link = dir.path().join("links/conflict");
        let wrong_src = dir.path().join("sources/wrong");

        write_source(&linked_src);
        write_source(&conflict_src);
        write_source(&wrong_src);
        fs::create_dir_all(linked_link.parent().expect("link parent")).expect("create link parent");
        symlink(&linked_src, &linked_link).expect("create linked symlink");
        symlink(&wrong_src, &conflict_link).expect("create conflict symlink");
        write_config(
            &config_path,
            vec![
                LinkEntry::new(&linked_link, &linked_src),
                LinkEntry::new(&missing_link, &missing_src),
                LinkEntry::new(&conflict_link, &conflict_src),
            ],
        );

        let items = list_config(&config_path).expect("list config");

        assert_eq!(
            items.iter().map(|item| item.status).collect::<Vec<_>>(),
            vec![
                LinkStatus::Conflict,
                LinkStatus::Linked,
                LinkStatus::Missing
            ]
        );
    }
}
// LCOV_EXCL_STOP
