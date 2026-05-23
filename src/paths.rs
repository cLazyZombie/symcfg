use std::{
    io,
    path::{Component, Path, PathBuf},
};

use crate::config::ConfigFile;

pub(crate) fn normalize_absolute_lexical(path: &Path) -> PathBuf {
    debug_assert!(path.is_absolute());

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }

    normalized
}

pub(crate) fn absolute_lexical(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(normalize_absolute_lexical(path))
    } else {
        std::env::current_dir().map(|cwd| normalize_absolute_lexical(&cwd.join(path)))
    }
}

pub(crate) fn resolve_symlink_target_lexical(link: &Path, target: &Path) -> io::Result<PathBuf> {
    if target.is_absolute() {
        Ok(normalize_absolute_lexical(target))
    } else if let Some(parent) = link.parent() {
        absolute_lexical(&parent.join(target))
    } else {
        absolute_lexical(target)
    }
}

pub(crate) fn normalize_config_entries(config: &mut ConfigFile) -> io::Result<()> {
    for entry in &mut config.links {
        entry.link = absolute_lexical(&entry.link)?;
        entry.src = absolute_lexical(&entry.src)?;
    }

    Ok(())
}
