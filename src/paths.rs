use std::{
    ffi::OsStr,
    fs,
    io::{self, ErrorKind},
    path::{Component, Path, PathBuf},
};

use crate::config::ConfigFile;

pub(crate) fn normalize_absolute_lexical(path: &Path) -> PathBuf {
    debug_assert!(path.is_absolute());

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()), // LCOV_EXCL_LINE
            Component::RootDir => normalized.push(component.as_os_str()),
            // LCOV_EXCL_START
            Component::CurDir => {}
            // LCOV_EXCL_STOP
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }

    normalized
}

fn home_dir() -> io::Result<PathBuf> {
    match std::env::var_os("HOME") {
        Some(home) if !home.as_os_str().is_empty() => Ok(PathBuf::from(home)),
        // LCOV_EXCL_START
        _ => Err(io::Error::new(ErrorKind::InvalidInput, "HOME is not set")),
        // LCOV_EXCL_STOP
    }
}

fn normalize_home_path(path: PathBuf) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(normalize_absolute_lexical(&path))
    } else {
        // LCOV_EXCL_START
        std::env::current_dir().map(|cwd| normalize_absolute_lexical(&cwd.join(path)))
        // LCOV_EXCL_STOP
    }
}

fn expand_home_marker(path: &Path) -> io::Result<Option<PathBuf>> {
    let mut components = path.components();

    let Some(Component::Normal(first)) = components.next() else {
        return Ok(None);
    };

    if first != OsStr::new("~") {
        return Ok(None);
    }

    let mut expanded = home_dir()?;
    for component in components {
        expanded.push(component.as_os_str());
    }

    normalize_home_path(expanded).map(Some)
}

pub(crate) fn absolute_lexical(path: &Path) -> io::Result<PathBuf> {
    if let Some(expanded) = expand_home_marker(path)? {
        Ok(expanded)
    } else if path.is_absolute() {
        Ok(normalize_absolute_lexical(path))
    } else {
        std::env::current_dir().map(|cwd| normalize_absolute_lexical(&cwd.join(path)))
    }
}

pub(crate) fn collapse_home_path(path: &Path) -> io::Result<PathBuf> {
    if !path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    let home = normalize_home_path(home_dir()?)?;
    let normalized = normalize_absolute_lexical(path);

    let Ok(relative) = normalized.strip_prefix(&home) else {
        return Ok(path.to_path_buf());
    };

    if relative.as_os_str().is_empty() {
        Ok(PathBuf::from("~"))
    } else {
        Ok(PathBuf::from("~").join(relative))
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

pub(crate) fn expand_config_home_markers(config: &mut ConfigFile) -> io::Result<()> {
    for entry in &mut config.links {
        if let Some(link) = expand_home_marker(&entry.link)? {
            entry.link = link;
        }
        if let Some(src) = expand_home_marker(&entry.src)? {
            entry.src = src;
        }
    }

    Ok(())
}

pub(crate) fn collapse_config_home_paths(config: &mut ConfigFile) -> io::Result<()> {
    for entry in &mut config.links {
        entry.link = collapse_home_path(&entry.link)?;
        entry.src = collapse_src_path(&entry.src)?;
    }

    Ok(())
}

fn collapse_src_path(path: &Path) -> io::Result<PathBuf> {
    if !path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    let normalized = normalize_absolute_lexical(path);
    let cwd = std::env::current_dir().map(|cwd| normalize_absolute_lexical(&cwd))?;

    if let Some(relative) = strip_prefix_or_dot(&normalized, &cwd) {
        return Ok(relative);
    }

    if let (Ok(canonical_path), Ok(canonical_cwd)) =
        (fs::canonicalize(&normalized), fs::canonicalize(&cwd))
        && let Some(relative) = strip_prefix_or_dot(&canonical_path, &canonical_cwd)
    {
        return Ok(relative);
    }

    collapse_home_path(&normalized)
}

fn strip_prefix_or_dot(path: &Path, base: &Path) -> Option<PathBuf> {
    let relative = path.strip_prefix(base).ok()?;

    if relative.as_os_str().is_empty() {
        Some(PathBuf::from("."))
    } else {
        Some(relative.to_path_buf())
    }
}

pub(crate) fn paths_equivalent(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    matches!(
        (fs::canonicalize(left), fs::canonicalize(right)),
        (Ok(left), Ok(right)) if left == right
    )
}

pub(crate) fn normalize_config_entries(config: &mut ConfigFile) -> io::Result<()> {
    for entry in &mut config.links {
        entry.link = absolute_lexical(&entry.link)?;
        entry.src = absolute_lexical(&entry.src)?;
    }

    Ok(())
}
// LCOV_EXCL_START
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_absolute_lexical_ignores_current_dir_components() {
        let normalized = normalize_absolute_lexical(Path::new("/tmp/./symcfg/../target"));

        assert_eq!(normalized, PathBuf::from("/tmp/target"));
    }

    #[test]
    fn resolve_symlink_target_lexical_resolves_relative_target_without_link_parent() {
        let target = Path::new("target-file");

        let resolved =
            resolve_symlink_target_lexical(Path::new(""), target).expect("resolve target");

        assert!(resolved.is_absolute());
        assert!(resolved.ends_with(target));
    }

    fn home_dir() -> PathBuf {
        PathBuf::from(std::env::var_os("HOME").expect("HOME must be set for path tests"))
    }

    fn home_relative_json_fragment(path: &Path) -> String {
        let home = home_dir();
        let relative = path
            .strip_prefix(&home)
            .expect("test path should be under HOME");
        format!("~/{}", relative.to_string_lossy())
    }

    #[test]
    fn absolute_lexical_expands_home_marker_for_operations() {
        let home = home_dir();

        assert_eq!(absolute_lexical(Path::new("~")).expect("expand home"), home);
        assert_eq!(
            absolute_lexical(Path::new("~/symcfg/../portable")).expect("expand home child"),
            home_dir().join("portable")
        );
    }

    #[test]
    fn normalize_config_entries_expands_home_marker_paths_for_operations() {
        let home = home_dir();
        let mut config = ConfigFile {
            version: crate::config::CURRENT_VERSION,
            links: vec![crate::config::LinkEntry::new(
                "~/links/editor.toml",
                "~/sources/editor.toml",
            )],
        };

        normalize_config_entries(&mut config).expect("normalize config entries");

        assert_eq!(
            config.links,
            vec![crate::config::LinkEntry::new(
                home.join("links/editor.toml"),
                home.join("sources/editor.toml"),
            )]
        );
    }

    #[test]
    fn collapse_home_path_collapses_home_directory_to_home_marker() {
        assert_eq!(
            collapse_home_path(&home_dir()).expect("collapse home"),
            PathBuf::from("~")
        );
    }

    #[test]
    fn saving_config_collapses_home_paths_to_home_marker() {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let path = dir.path().join("symbolic.json");
        let home = home_dir();
        let link = home.join("links/editor.toml");
        let src = home.join("sources/editor.toml");
        let mut config = ConfigFile {
            version: crate::config::CURRENT_VERSION,
            links: vec![crate::config::LinkEntry::new(&link, &src)],
        };

        config.save(&path).expect("save config");

        let actual = std::fs::read_to_string(&path).expect("read saved config");
        assert!(actual.contains(&format!(
            "\"link\": \"{}\"",
            home_relative_json_fragment(&link)
        )));
        assert!(actual.contains(&format!(
            "\"src\": \"{}\"",
            home_relative_json_fragment(&src)
        )));
        assert!(!actual.contains(home.to_str().expect("utf-8 home path")));
    }
}
// LCOV_EXCL_STOP
