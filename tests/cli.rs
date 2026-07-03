use std::{
    fs,
    os::unix::fs::{self as unix_fs, PermissionsExt},
    path::Path,
};

use assert_cmd::Command;
use assert_fs::TempDir;
use predicates::prelude::*;
use serde_json::{Value, json};

const DEFAULT_CONFIG: &str = "symbolic.json";

fn symcfg() -> Command {
    Command::cargo_bin("symcfg").expect("symcfg binary is built for integration tests")
}

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create file parent");
    }
    fs::write(path, contents).expect("write test file");
}

fn write_config(path: &Path, links: &[(&Path, &Path)]) {
    let links: Vec<Value> = links
        .iter()
        .map(|(link, src)| json!({ "link": link, "src": src }))
        .collect();
    let config = json!({ "version": 1, "links": links });

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create config parent");
    }
    fs::write(
        path,
        serde_json::to_string_pretty(&config).expect("serialize config"),
    )
    .expect("write config");
}

fn read_config(path: &Path) -> Value {
    let contents = fs::read_to_string(path).expect("read config");
    serde_json::from_str(&contents).expect("parse config")
}

fn links(config: &Value) -> &[Value] {
    config["links"].as_array().expect("config links array")
}

fn assert_has_entry(config_path: &Path, link: &Path, src: &Path) {
    let expected_src = src.to_string_lossy();
    assert_has_entry_src(config_path, link, expected_src.as_ref());
}

fn assert_has_entry_src(config_path: &Path, link: &Path, src: &str) {
    let config = read_config(config_path);
    let expected_link = link.to_string_lossy();

    assert!(
        links(&config).iter().any(|entry| {
            entry["link"].as_str() == Some(expected_link.as_ref())
                && entry["src"].as_str() == Some(src)
                && entry.get("target").is_none()
        }),
        "config {config_path:?} should contain link={link:?}, src={src:?}, and no target field; actual: {config}"
    );
}

fn assert_no_entry(config_path: &Path, link: &Path) {
    let config = read_config(config_path);
    let expected_link = link.to_string_lossy();

    assert!(
        links(&config)
            .iter()
            .all(|entry| entry["link"].as_str() != Some(expected_link.as_ref())),
        "config {config_path:?} should not contain link={link:?}; actual: {config}"
    );
}

fn assert_symlink_points_to(link: &Path, src: &Path) {
    let actual = fs::read_link(link).expect("read symlink target");
    if actual == src {
        return;
    }

    if let (Ok(actual), Ok(expected)) = (actual.canonicalize(), src.canonicalize()) {
        assert_eq!(actual, expected);
        return;
    }

    assert_eq!(actual, src);
}

fn assert_symlink_exists(path: &Path) {
    let metadata = fs::symlink_metadata(path).expect("symlink metadata");
    assert!(
        metadata.file_type().is_symlink(),
        "{path:?} should be a symlink"
    );
}

fn label(text: &str) -> String {
    format!("{text:<18}")
}

fn item_line(status: &str, link: &Path, src: &Path) -> String {
    format!(
        "{} {} -> {}\n",
        label(status),
        link.display(),
        src.display()
    )
}

fn path_line(status: &str, path: &Path) -> String {
    format!("{} {}\n", label(status), path.display())
}

fn summary_line(status: &str, counts: &str) -> String {
    format!("{} {counts}\n", label(status))
}

#[test]
fn search_writes_default_symbolic_json_with_link_and_src_entries() {
    let temp = TempDir::new().expect("create temporary directory");
    let source_root = temp.path().join("sources");
    let link_root = temp.path().join("links");
    let src = source_root.join("app/settings.toml");
    let link = link_root.join("app/settings.toml");
    write_file(&src, "theme = 'dark'\n");
    fs::create_dir_all(link.parent().expect("link parent")).expect("create link parent");
    unix_fs::symlink(&src, &link).expect("create symlink");

    symcfg()
        .current_dir(temp.path())
        .args([
            "search",
            "--source",
            source_root.to_str().expect("utf-8 source path"),
            link_root.to_str().expect("utf-8 link path"),
        ])
        .assert()
        .success()
        .stdout(format!(
            "{}{}",
            item_line("added", &link, &src),
            summary_line(
                "Search complete",
                "matched=1, added=1, duplicate=0, conflict=0"
            )
        ));

    let config = temp.path().join(DEFAULT_CONFIG);
    assert!(config.exists(), "search should create symbolic.json in cwd");
    assert_has_entry_src(&config, &link, "sources/app/settings.toml");
}

#[test]
fn search_writes_custom_output_path() {
    let temp = TempDir::new().expect("create temporary directory");
    let source_root = temp.path().join("sources");
    let link_root = temp.path().join("links");
    let custom_config = temp.path().join("config/custom-symbolic.json");
    let src = source_root.join("shell/profile");
    let link = link_root.join("profile");
    write_file(&src, "export EDITOR=vi\n");
    fs::create_dir_all(&link_root).expect("create link root");
    unix_fs::symlink(&src, &link).expect("create symlink");

    symcfg()
        .current_dir(temp.path())
        .args([
            "search",
            "--source",
            source_root.to_str().expect("utf-8 source path"),
            link_root.to_str().expect("utf-8 link path"),
            "--output",
            custom_config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .success();

    assert!(
        custom_config.exists(),
        "search should create custom config path"
    );
    assert!(
        !temp.path().join(DEFAULT_CONFIG).exists(),
        "custom output should not create default config"
    );
    assert_has_entry_src(&custom_config, &link, "sources/shell/profile");
}

#[test]
fn search_prints_duplicate_and_conflict_items() {
    let temp = TempDir::new().expect("create temporary directory");
    let source_root = temp.path().join("sources");
    let link_root = temp.path().join("links");
    let config = temp.path().join("symbolic.json");
    let duplicate_src = source_root.join("duplicate");
    let conflict_src = source_root.join("conflict");
    let previous_src = source_root.join("previous");
    let duplicate_link = link_root.join("duplicate");
    let conflict_link = link_root.join("conflict");
    write_file(&duplicate_src, "duplicate\n");
    write_file(&conflict_src, "conflict\n");
    write_file(&previous_src, "previous\n");
    fs::create_dir_all(&link_root).expect("create link root");
    unix_fs::symlink(&duplicate_src, &duplicate_link).expect("create duplicate symlink");
    unix_fs::symlink(&conflict_src, &conflict_link).expect("create conflict symlink");
    write_config(
        &config,
        &[
            (&duplicate_link, &duplicate_src),
            (&conflict_link, &previous_src),
        ],
    );

    symcfg()
        .current_dir(temp.path())
        .args([
            "search",
            "--source",
            source_root.to_str().expect("utf-8 source path"),
            link_root.to_str().expect("utf-8 link path"),
            "--output",
            config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("duplicate")
                .and(predicate::str::contains("conflict"))
                .and(predicate::str::contains("existing="))
                .and(predicate::str::contains("new="))
                .and(predicate::str::contains(
                    "Search complete    matched=2, added=0, duplicate=1, conflict=1",
                )),
        );
}

#[test]
fn link_yes_creates_missing_parent_symlink_and_registers_entry() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let src = temp.path().join("sources/editor.toml");
    let link = temp.path().join("links/missing/editor.toml");
    write_file(&src, "tab_width = 4\n");

    symcfg()
        .current_dir(temp.path())
        .args([
            "link",
            src.to_str().expect("utf-8 source path"),
            link.to_str().expect("utf-8 link path"),
            "--yes",
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .success()
        .stdout(format!(
            "{}{}{}{}",
            path_line("created-parent", link.parent().expect("link parent")),
            item_line("created", &link, &src),
            item_line("registered", &link, &src),
            summary_line(
                "Link complete",
                "created=true, parent_created=true, registered=true, duplicate=false"
            )
        ));

    assert_symlink_points_to(&link, &src);
    assert_has_entry_src(&config, &link, "sources/editor.toml");
}

#[test]
fn link_yes_writes_home_relative_link_and_current_dir_relative_src() {
    let temp = TempDir::new().expect("create temporary directory");
    let fake_home = temp.path().join("home/user");
    let config = fake_home.join("config/symbolic.json");
    let src = fake_home.join("sources/editor.toml");
    let link = fake_home.join("links/missing/editor.toml");
    write_file(&src, "tab_width = 4\n");

    symcfg()
        .current_dir(&fake_home)
        .args([
            "link",
            src.to_str().expect("utf-8 source path"),
            link.to_str().expect("utf-8 link path"),
            "--yes",
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .env("HOME", &fake_home)
        .assert()
        .success()
        .stdout(format!(
            "{}{}{}{}",
            path_line("created-parent", link.parent().expect("link parent")),
            item_line("created", &link, &src),
            item_line("registered", &link, &src),
            summary_line(
                "Link complete",
                "created=true, parent_created=true, registered=true, duplicate=false"
            )
        ));

    assert_symlink_points_to(&link, &src);

    let raw = fs::read_to_string(&config).expect("read config");
    let expected_link = format!(
        "~/{}",
        link.strip_prefix(&fake_home)
            .expect("link should be under fake home")
            .to_string_lossy()
    );

    assert!(raw.contains(&format!("\"link\": \"{expected_link}\"")));
    assert!(raw.contains("\"src\": \"sources/editor.toml\""));
    assert!(!raw.contains(fake_home.to_str().expect("utf-8 fake home path")));
}

#[test]
fn link_yes_writes_dot_when_src_is_current_directory() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let link = temp.path().join("links/root");

    symcfg()
        .current_dir(temp.path())
        .args([
            "link",
            temp.path().to_str().expect("utf-8 source path"),
            link.to_str().expect("utf-8 link path"),
            "--yes",
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .success()
        .stdout(format!(
            "{}{}{}{}",
            path_line("created-parent", link.parent().expect("link parent")),
            item_line("created", &link, temp.path()),
            item_line("registered", &link, temp.path()),
            summary_line(
                "Link complete",
                "created=true, parent_created=true, registered=true, duplicate=false"
            )
        ));

    assert_symlink_points_to(&link, temp.path());

    let raw = fs::read_to_string(&config).expect("read config");
    assert!(raw.contains("\"src\": \".\""));
}

#[test]
fn link_yes_prints_already_linked_and_duplicate_when_entry_exists() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let src = temp.path().join("sources/editor.toml");
    let link = temp.path().join("links/editor.toml");
    write_file(&src, "tab_width = 4\n");
    fs::create_dir_all(link.parent().expect("link parent")).expect("create link parent");
    unix_fs::symlink(&src, &link).expect("create existing symlink");
    write_config(&config, &[(&link, &src)]);

    symcfg()
        .current_dir(temp.path())
        .args([
            "link",
            src.to_str().expect("utf-8 source path"),
            link.to_str().expect("utf-8 link path"),
            "--yes",
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .success()
        .stdout(format!(
            "{}{}{}",
            item_line("skipped:already-linked", &link, &src),
            item_line("duplicate", &link, &src),
            summary_line(
                "Link complete",
                "created=false, parent_created=false, registered=false, duplicate=true"
            )
        ));
}

#[test]
fn import_yes_copies_existing_file_replaces_it_with_symlink_and_registers_entry() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let link = temp.path().join("app/zshrc");
    let src = temp.path().join("sources/zshrc");
    write_file(&link, "setopt prompt_subst\n");

    symcfg()
        .current_dir(temp.path())
        .args([
            "import",
            link.to_str().expect("utf-8 link path"),
            src.to_str().expect("utf-8 source path"),
            "--yes",
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .success()
        .stdout(format!(
            "{}{}{}{}",
            path_line("created-parent", src.parent().expect("source parent")),
            item_line("imported", &link, &src),
            item_line("registered", &link, &src),
            summary_line(
                "Import complete",
                "imported=true, parent_created=true, registered=true, duplicate=false"
            )
        ));

    assert_symlink_points_to(&link, &src);
    assert_eq!(
        fs::read_to_string(&src).expect("read imported source"),
        "setopt prompt_subst\n"
    );
    assert_has_entry_src(&config, &link, "sources/zshrc");
}

#[test]
fn import_yes_prints_duplicate_when_entry_already_exists() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let link = temp.path().join("app/gitconfig");
    let src = temp.path().join("sources/gitconfig");
    write_file(&link, "[user]\n\tname = Example\n");
    write_config(&config, &[(&link, &src)]);

    symcfg()
        .current_dir(temp.path())
        .args([
            "import",
            link.to_str().expect("utf-8 link path"),
            src.to_str().expect("utf-8 source path"),
            "--yes",
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .success()
        .stdout(format!(
            "{}{}{}{}",
            path_line("created-parent", src.parent().expect("source parent")),
            item_line("imported", &link, &src),
            item_line("duplicate", &link, &src),
            summary_line(
                "Import complete",
                "imported=true, parent_created=true, registered=false, duplicate=true"
            )
        ));

    assert_symlink_points_to(&link, &src);
    assert_has_entry_src(&config, &link, "sources/gitconfig");
}

#[test]
fn import_yes_preserves_original_file_permissions() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let link = temp.path().join("app/secret.conf");
    let src = temp.path().join("sources/secret.conf");
    write_file(&link, "token = secret\n");
    fs::set_permissions(&link, fs::Permissions::from_mode(0o600))
        .expect("restrict original file permissions");

    symcfg()
        .current_dir(temp.path())
        .args([
            "import",
            link.to_str().expect("utf-8 link path"),
            src.to_str().expect("utf-8 source path"),
            "--yes",
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .success();

    assert_symlink_points_to(&link, &src);
    assert_eq!(
        fs::metadata(&src)
            .expect("read imported source metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn import_prompts_in_english_and_declines_without_mutating_filesystem() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let link = temp.path().join("app/tmux.conf");
    let src = temp.path().join("sources/tmux.conf");
    write_file(&link, "set -g mouse on\n");

    symcfg()
        .current_dir(temp.path())
        .args([
            "import",
            link.to_str().expect("utf-8 link path"),
            src.to_str().expect("utf-8 source path"),
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .write_stdin("n\n")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Import existing file")
                .and(predicate::str::contains("replace it with a symlink"))
                .and(predicate::str::contains("skipped:declined"))
                .and(predicate::str::contains(
                    "Import complete    imported=false, parent_created=false, registered=false, duplicate=false",
                )),
        );

    assert_eq!(
        fs::read_to_string(&link).expect("read original link file"),
        "set -g mouse on\n"
    );
    assert!(!src.exists());
    assert!(!config.exists());
}

#[test]
fn import_prompts_in_english_and_accepts_yes_with_existing_source_parent() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let link = temp.path().join("app/starship.toml");
    let src = temp.path().join("sources/starship.toml");
    write_file(&link, "format = '$all'\n");
    fs::create_dir_all(src.parent().expect("source parent")).expect("create source parent");

    symcfg()
        .current_dir(temp.path())
        .args([
            "import",
            link.to_str().expect("utf-8 link path"),
            src.to_str().expect("utf-8 source path"),
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .write_stdin("y\n")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Import existing file")
                .and(predicate::str::contains("imported"))
                .and(predicate::str::contains(
                    "Import complete    imported=true, parent_created=false, registered=true, duplicate=false",
                )),
        );

    assert_symlink_points_to(&link, &src);
    assert_eq!(
        fs::read_to_string(&src).expect("read imported source"),
        "format = '$all'\n"
    );
    assert_has_entry_src(&config, &link, "sources/starship.toml");
}

#[test]
fn import_fails_when_source_already_exists_without_overwriting_anything() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let link = temp.path().join("app/config.toml");
    let src = temp.path().join("sources/config.toml");
    write_file(&link, "from app\n");
    write_file(&src, "from source\n");

    symcfg()
        .current_dir(temp.path())
        .args([
            "import",
            link.to_str().expect("utf-8 link path"),
            src.to_str().expect("utf-8 source path"),
            "--yes",
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("source path already exists"));

    assert_eq!(
        fs::read_to_string(&link).expect("read link file"),
        "from app\n"
    );
    assert_eq!(
        fs::read_to_string(&src).expect("read source file"),
        "from source\n"
    );
    assert!(!config.exists());
}

#[test]
fn import_rejects_source_path_that_is_the_config_file() {
    let temp = TempDir::new().expect("create temporary directory");
    let link = temp.path().join("app/zshrc");
    let config = temp.path().join(DEFAULT_CONFIG);
    write_file(&link, "setopt prompt_subst\n");

    symcfg()
        .current_dir(temp.path())
        .args([
            "import",
            link.to_str().expect("utf-8 link path"),
            DEFAULT_CONFIG,
            "--yes",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "source path must not be the config file",
        ));

    assert_eq!(
        fs::read_to_string(&link).expect("read original link file"),
        "setopt prompt_subst\n"
    );
    assert!(!config.exists());
}

#[test]
fn import_rejects_source_path_that_aliases_config_file() {
    let temp = TempDir::new().expect("create temporary directory");
    let real = temp.path().join("real");
    let alias = temp.path().join("alias");
    let link = temp.path().join("app/zshrc");
    let config = real.join(DEFAULT_CONFIG);
    let src = alias.join(DEFAULT_CONFIG);
    fs::create_dir_all(&real).expect("create real config parent");
    unix_fs::symlink(&real, &alias).expect("create aliased config parent");
    write_file(&link, "setopt prompt_subst\n");

    symcfg()
        .current_dir(temp.path())
        .args([
            "import",
            link.to_str().expect("utf-8 link path"),
            src.to_str().expect("utf-8 source path"),
            "--yes",
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "source path must not be the config file",
        ));

    assert_eq!(
        fs::read_to_string(&link).expect("read original link file"),
        "setopt prompt_subst\n"
    );
    assert!(!config.exists());
    assert!(!src.exists());
}

#[test]
fn import_rolls_back_source_copy_when_link_replacement_fails() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let link = temp.path().join("app/config.toml");
    let link_parent = link.parent().expect("link parent");
    let src = temp.path().join("sources/config.toml");
    write_file(&link, "from app\n");
    fs::create_dir_all(src.parent().expect("source parent")).expect("create source parent");

    let mut permissions = fs::metadata(link_parent)
        .expect("link parent metadata")
        .permissions();
    permissions.set_mode(0o555);
    fs::set_permissions(link_parent, permissions).expect("make link parent read-only");

    symcfg()
        .current_dir(temp.path())
        .args([
            "import",
            link.to_str().expect("utf-8 link path"),
            src.to_str().expect("utf-8 source path"),
            "--yes",
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("I/O error"));

    let mut permissions = fs::metadata(link_parent)
        .expect("link parent metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(link_parent, permissions).expect("restore link parent permissions");

    assert_eq!(
        fs::read_to_string(&link).expect("read original link file"),
        "from app\n"
    );
    assert!(!src.exists());
    assert!(!config.exists());
}

#[test]
fn import_fails_when_link_is_missing_symlink_or_directory() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let missing_link = temp.path().join("app/missing.conf");
    let symlink_link = temp.path().join("app/symlink.conf");
    let directory_link = temp.path().join("app/config-dir");
    let existing_target = temp.path().join("target.conf");
    let src = temp.path().join("sources/config.conf");
    write_file(&existing_target, "target\n");
    fs::create_dir_all(symlink_link.parent().expect("symlink parent")).expect("create app dir");
    unix_fs::symlink(&existing_target, &symlink_link).expect("create existing symlink");
    fs::create_dir_all(&directory_link).expect("create directory link path");

    symcfg()
        .current_dir(temp.path())
        .args([
            "import",
            missing_link.to_str().expect("utf-8 missing link path"),
            src.to_str().expect("utf-8 source path"),
            "--yes",
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("link path does not exist"));

    symcfg()
        .current_dir(temp.path())
        .args([
            "import",
            symlink_link.to_str().expect("utf-8 symlink path"),
            src.to_str().expect("utf-8 source path"),
            "--yes",
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already a symlink"));

    symcfg()
        .current_dir(temp.path())
        .args([
            "import",
            directory_link.to_str().expect("utf-8 directory path"),
            src.to_str().expect("utf-8 source path"),
            "--yes",
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a regular file"));

    assert!(!src.exists());
    assert!(!config.exists());
    assert_symlink_points_to(&symlink_link, &existing_target);
    assert!(directory_link.is_dir());
}

#[test]
fn link_prompts_in_english_for_missing_parent_and_accepts_yes_on_stdin() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let src = temp.path().join("sources/gitconfig");
    let link = temp.path().join("links/missing/gitconfig");
    write_file(&src, "[user]\n\tname = Example\n");

    symcfg()
        .current_dir(temp.path())
        .args([
            "link",
            src.to_str().expect("utf-8 source path"),
            link.to_str().expect("utf-8 link path"),
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .write_stdin("y\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("Create").and(predicate::str::contains("parent")));

    assert_symlink_points_to(&link, &src);
    assert_has_entry_src(&config, &link, "sources/gitconfig");
}

#[test]
fn link_prompts_for_missing_parent_and_declines_no_on_stdin() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let src = temp.path().join("sources/gitconfig");
    let link = temp.path().join("links/missing/gitconfig");
    write_file(&src, "[user]\n\tname = Example\n");

    symcfg()
        .current_dir(temp.path())
        .args([
            "link",
            src.to_str().expect("utf-8 source path"),
            link.to_str().expect("utf-8 link path"),
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .write_stdin("n\n")
        .assert()
        .failure()
        .stdout(predicate::str::contains("Create").and(predicate::str::contains("parent")))
        .stderr(predicate::str::contains(
            "parent directory creation declined",
        ));

    assert!(!link.parent().expect("link parent").exists());
    assert!(!link.exists());
    assert!(!config.exists());
}

#[test]
fn apply_yes_creates_missing_symlinks_and_prints_english_summary_counts() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let src = temp.path().join("sources/zshrc");
    let link = temp.path().join("links/zshrc");
    write_file(&src, "setopt prompt_subst\n");
    write_config(&config, &[(&link, &src)]);
    fs::create_dir_all(link.parent().expect("link parent")).expect("create link parent");

    symcfg()
        .current_dir(temp.path())
        .args([
            "apply",
            "--config",
            config.to_str().expect("utf-8 config path"),
            "--yes",
        ])
        .assert()
        .success()
        .stdout(format!(
            "{}{}",
            item_line("created", &link, &src),
            summary_line("Apply complete", "created=1, skipped=0, conflict=0")
        ));

    assert_symlink_points_to(&link, &src);
}

#[test]
fn apply_yes_resolves_relative_src_from_current_directory() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let src = temp.path().join("sources/zshrc");
    let link = temp.path().join("links/zshrc");
    write_file(&src, "setopt prompt_subst\n");
    write_config(&config, &[(&link, Path::new("sources/zshrc"))]);
    fs::create_dir_all(link.parent().expect("link parent")).expect("create link parent");

    symcfg()
        .current_dir(temp.path())
        .args([
            "apply",
            "--config",
            config.to_str().expect("utf-8 config path"),
            "--yes",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("created")
                .and(predicate::str::contains(
                    link.to_str().expect("utf-8 link path"),
                ))
                .and(predicate::str::contains(
                    "Apply complete     created=1, skipped=0, conflict=0",
                )),
        );

    assert_symlink_points_to(&link, &src);
}

#[test]
fn apply_yes_skips_link_when_parent_directory_is_missing() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let src = temp.path().join("sources/gitconfig");
    let link = temp.path().join("links/gitconfig");
    write_file(&src, "[user]\n");
    write_config(&config, &[(&link, &src)]);

    symcfg()
        .current_dir(temp.path())
        .args([
            "apply",
            "--config",
            config.to_str().expect("utf-8 config path"),
            "--yes",
        ])
        .assert()
        .success()
        .stdout(format!(
            "{}{}",
            item_line("skipped:missing-parent", &link, &src),
            summary_line("Apply complete", "created=0, skipped=1, conflict=0")
        ));

    assert!(!link.parent().expect("link parent").exists());
    assert!(!link.exists());
}

#[test]
fn apply_yes_prints_already_linked_and_conflict_items() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let linked_src = temp.path().join("sources/linked");
    let conflict_src = temp.path().join("sources/conflict");
    let linked_link = temp.path().join("links/linked");
    let conflict_link = temp.path().join("links/conflict");
    write_file(&linked_src, "linked\n");
    write_file(&conflict_src, "conflict\n");
    fs::create_dir_all(linked_link.parent().expect("link parent")).expect("create link parent");
    unix_fs::symlink(&linked_src, &linked_link).expect("create linked symlink");
    fs::write(&conflict_link, "not a symlink\n").expect("write regular conflict path");
    write_config(
        &config,
        &[(&linked_link, &linked_src), (&conflict_link, &conflict_src)],
    );

    symcfg()
        .current_dir(temp.path())
        .args([
            "apply",
            "--config",
            config.to_str().expect("utf-8 config path"),
            "--yes",
        ])
        .assert()
        .success()
        .stdout(format!(
            "{}{}{}",
            item_line("conflict", &conflict_link, &conflict_src),
            item_line("skipped:already-linked", &linked_link, &linked_src),
            summary_line("Apply complete", "created=0, skipped=1, conflict=1")
        ));
}

#[test]
fn apply_prompts_in_english_before_creating_link_and_accepts_yes_on_stdin() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let src = temp.path().join("sources/tmux.conf");
    let link = temp.path().join("links/tmux.conf");
    write_file(&src, "set -g mouse on\n");
    write_config(&config, &[(&link, &src)]);
    fs::create_dir_all(link.parent().expect("link parent")).expect("create link parent");

    symcfg()
        .current_dir(temp.path())
        .args([
            "apply",
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .write_stdin("y\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("Create").and(predicate::str::contains("link")));

    assert_symlink_points_to(&link, &src);
}

#[test]
fn apply_prompts_for_missing_link_and_declines_no_on_stdin() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let src = temp.path().join("sources/tmux.conf");
    let link = temp.path().join("links/tmux.conf");
    write_file(&src, "set -g mouse on\n");
    write_config(&config, &[(&link, &src)]);
    fs::create_dir_all(link.parent().expect("link parent")).expect("create link parent");

    symcfg()
        .current_dir(temp.path())
        .args([
            "apply",
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .write_stdin("n\n")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Create link").and(predicate::str::contains(
                "Apply complete     created=0, skipped=1, conflict=0",
            )),
        );

    assert!(!link.exists());
}

#[test]
fn sync_yes_without_delete_policy_fails_with_english_error() {
    let temp = TempDir::new().expect("create temporary directory");
    let source_root = temp.path().join("sources");
    let config = temp.path().join("symbolic.json");
    let stale_src = source_root.join("removed.conf");
    let stale_link = temp.path().join("links/removed.conf");
    fs::create_dir_all(&source_root).expect("create source root");
    write_config(&config, &[(&stale_link, &stale_src)]);

    symcfg()
        .current_dir(temp.path())
        .args([
            "sync",
            source_root.to_str().expect("utf-8 source path"),
            "--config",
            config.to_str().expect("utf-8 config path"),
            "--yes",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("delete").and(predicate::str::contains("keep-links")));
}

#[test]
fn sync_keep_links_removes_stale_entries_keeps_link_and_prints_summary() {
    let temp = TempDir::new().expect("create temporary directory");
    let source_root = temp.path().join("sources");
    let config = temp.path().join("symbolic.json");
    let stale_src = source_root.join("removed.conf");
    let stale_link = temp.path().join("links/removed.conf");
    fs::create_dir_all(stale_link.parent().expect("stale link parent"))
        .expect("create link parent");
    fs::create_dir_all(&source_root).expect("create source root");
    unix_fs::symlink(&stale_src, &stale_link).expect("create stale symlink");
    write_config(&config, &[(&stale_link, &stale_src)]);

    symcfg()
        .current_dir(temp.path())
        .args([
            "sync",
            source_root.to_str().expect("utf-8 source path"),
            "--config",
            config.to_str().expect("utf-8 config path"),
            "--yes",
            "--keep-links",
        ])
        .assert()
        .success()
        .stdout(format!(
            "{}{}",
            item_line("kept", &stale_link, &stale_src),
            summary_line("Sync complete", "stale=1, removed=1, deleted=0, kept=1")
        ));

    assert_no_entry(&config, &stale_link);
    assert_symlink_exists(&stale_link);
}

#[test]
fn sync_delete_links_removes_stale_entries_and_deletes_only_matching_symlink() {
    let temp = TempDir::new().expect("create temporary directory");
    let source_root = temp.path().join("sources");
    let config = temp.path().join("symbolic.json");
    let stale_src = source_root.join("removed.conf");
    let matching_link = temp.path().join("links/removed.conf");
    let nonmatching_link = temp.path().join("links/not-removed.conf");
    let unrelated_target = temp.path().join("elsewhere/actual.conf");
    fs::create_dir_all(matching_link.parent().expect("matching link parent"))
        .expect("create link parent");
    fs::create_dir_all(&source_root).expect("create source root");
    write_file(&unrelated_target, "still here\n");
    unix_fs::symlink(&stale_src, &matching_link).expect("create matching stale symlink");
    unix_fs::symlink(&unrelated_target, &nonmatching_link).expect("create nonmatching symlink");
    write_config(
        &config,
        &[
            (&matching_link, &stale_src),
            (&nonmatching_link, &stale_src),
        ],
    );

    symcfg()
        .current_dir(temp.path())
        .args([
            "sync",
            source_root.to_str().expect("utf-8 source path"),
            "--config",
            config.to_str().expect("utf-8 config path"),
            "--yes",
            "--delete-links",
        ])
        .assert()
        .success()
        .stdout(format!(
            "{}{}{}",
            item_line("kept", &nonmatching_link, &stale_src),
            item_line("deleted", &matching_link, &stale_src),
            summary_line("Sync complete", "stale=2, removed=2, deleted=1, kept=1")
        ));

    assert_no_entry(&config, &matching_link);
    assert_no_entry(&config, &nonmatching_link);
    assert!(
        fs::symlink_metadata(&matching_link).is_err(),
        "matching stale symlink should be deleted"
    );
    assert_symlink_points_to(&nonmatching_link, &unrelated_target);
}

#[test]
fn sync_delete_links_prints_missing_link_item() {
    let temp = TempDir::new().expect("create temporary directory");
    let source_root = temp.path().join("sources");
    let config = temp.path().join("symbolic.json");
    let stale_src = source_root.join("removed.conf");
    let stale_link = temp.path().join("links/removed.conf");
    fs::create_dir_all(&source_root).expect("create source root");
    write_config(&config, &[(&stale_link, &stale_src)]);

    symcfg()
        .current_dir(temp.path())
        .args([
            "sync",
            source_root.to_str().expect("utf-8 source path"),
            "--config",
            config.to_str().expect("utf-8 config path"),
            "--yes",
            "--delete-links",
        ])
        .assert()
        .success()
        .stdout(format!(
            "{}{}",
            item_line("missing-link", &stale_link, &stale_src),
            summary_line("Sync complete", "stale=1, removed=1, deleted=0, kept=0")
        ));

    assert_no_entry(&config, &stale_link);
    assert!(!stale_link.exists());
}

#[test]
fn validate_prints_english_success_for_valid_config_and_failure_for_invalid_config() {
    let temp = TempDir::new().expect("create temporary directory");
    let valid_config = temp.path().join("valid.json");
    let schema_config = temp.path().join("schema-mismatch.json");
    let src = temp.path().join("sources/vimrc");
    let link = temp.path().join("links/vimrc");
    write_file(&src, "syntax on\n");
    write_config(&valid_config, &[(&link, &src)]);
    fs::write(
        &schema_config,
        serde_json::to_string_pretty(&json!({
            "version": 1,
            "links": [{ "link": link, "target": src }]
        }))
        .expect("serialize schema mismatch config"),
    )
    .expect("write schema mismatch config");

    symcfg()
        .current_dir(temp.path())
        .args([
            "validate",
            "--config",
            valid_config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("valid"));

    symcfg()
        .current_dir(temp.path())
        .args([
            "validate",
            "--config",
            schema_config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("invalid config")
                .and(predicate::str::contains("missing field `src`")),
        );
}

#[test]
fn validate_rejects_config_where_link_path_is_also_a_source_path() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    write_config(
        &config,
        &[
            (
                Path::new(".config/herdr/config.toml"),
                Path::new("herdr/config.toml"),
            ),
            (
                Path::new("herdr/config.toml"),
                Path::new("herdr/config.toml"),
            ),
        ],
    );

    symcfg()
        .current_dir(temp.path())
        .args([
            "validate",
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("invalid config")
                .and(predicate::str::contains(
                    "link path must not be the same as src path",
                ))
                .and(predicate::str::contains("herdr/config.toml")),
        );
}

#[test]
fn link_rejects_registering_an_existing_source_path_as_a_link_path() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let managed_link = temp.path().join(".config/herdr/config.toml");
    let managed_src = temp.path().join("herdr/config.toml");
    let other_src = temp.path().join("herdr/other.toml");
    write_file(&managed_src, "managed\n");
    write_file(&other_src, "other\n");
    write_config(&config, &[(&managed_link, Path::new("herdr/config.toml"))]);

    symcfg()
        .current_dir(temp.path())
        .args([
            "link",
            other_src.to_str().expect("utf-8 source path"),
            managed_src.to_str().expect("utf-8 link path"),
            "--yes",
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "link path must not also be used as a source path",
        ));

    assert_eq!(
        fs::read_to_string(&managed_src).expect("read managed source"),
        "managed\n"
    );
    assert_has_entry_src(&config, &managed_link, "herdr/config.toml");
    assert_no_entry(&config, &managed_src);
}

#[test]
fn help_output_explains_top_level_commands_and_import_details() {
    symcfg().args(["--help"]).assert().success().stdout(
        predicate::str::contains("Manage configuration files through symbolic links")
            .and(predicate::str::contains("import"))
            .and(predicate::str::contains("Import an existing regular file")),
    );

    symcfg()
        .args(["import", "--help"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Copy the existing regular file")
                .and(predicate::str::contains(
                    "refuses to overwrite an existing SRC",
                ))
                .and(predicate::str::contains(
                    "Existing regular file at the application path",
                )),
        );

    symcfg().args(["link", "--help"]).assert().success().stdout(
        predicate::str::contains("Create LINK as a symbolic link").and(predicate::str::contains(
            "Application path where the symbolic link",
        )),
    );
}

#[test]
fn list_prints_one_line_per_config_entry_with_link_status() {
    let temp = TempDir::new().expect("create temporary directory");
    let config = temp.path().join("symbolic.json");
    let linked_src = temp.path().join("sources/linked");
    let missing_src = Path::new("sources/missing");
    let conflict_src = temp.path().join("sources/conflict");
    let linked_link = temp.path().join("links/linked");
    let missing_link = temp.path().join("links/missing");
    let conflict_link = temp.path().join("links/conflict");
    let regular_link = temp.path().join("links/regular");
    let wrong_src = temp.path().join("sources/wrong");
    write_file(&linked_src, "linked\n");
    write_file(&conflict_src, "conflict\n");
    write_file(&wrong_src, "wrong\n");
    fs::create_dir_all(linked_link.parent().expect("link parent")).expect("create link parent");
    unix_fs::symlink(&linked_src, &linked_link).expect("create linked symlink");
    unix_fs::symlink(&wrong_src, &conflict_link).expect("create conflicting symlink");
    fs::write(&regular_link, "not a symlink\n").expect("write regular file at link path");
    write_config(
        &config,
        &[
            (&linked_link, Path::new("sources/linked")),
            (&missing_link, missing_src),
            (&conflict_link, Path::new("sources/conflict")),
            (&regular_link, Path::new("sources/regular")),
        ],
    );

    let expected = format!(
        "{} {} -> sources/conflict\n{} {} -> sources/linked\n{} {} -> sources/missing\n{} {} -> sources/regular\n",
        label("conflict"),
        conflict_link.display(),
        label("linked"),
        linked_link.display(),
        label("missing"),
        missing_link.display(),
        label("conflict"),
        regular_link.display()
    );

    symcfg()
        .current_dir(temp.path())
        .args([
            "list",
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .success()
        .stdout(expected);
}

#[test]
fn sync_prompts_in_english_for_stale_link_delete_and_accepts_yes_on_stdin() {
    let temp = TempDir::new().expect("create temporary directory");
    let source_root = temp.path().join("sources");
    let config = temp.path().join("symbolic.json");
    let stale_src = source_root.join("removed.conf");
    let stale_link = temp.path().join("links/removed.conf");
    fs::create_dir_all(stale_link.parent().expect("stale link parent"))
        .expect("create link parent");
    fs::create_dir_all(&source_root).expect("create source root");
    unix_fs::symlink(&stale_src, &stale_link).expect("create stale symlink");
    write_config(&config, &[(&stale_link, &stale_src)]);

    symcfg()
        .current_dir(temp.path())
        .args([
            "sync",
            source_root.to_str().expect("utf-8 source path"),
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .write_stdin("y\n")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("stale")
                .and(predicate::str::contains("Delete"))
                .and(predicate::str::contains("link")),
        );

    assert_no_entry(&config, &stale_link);
    assert!(
        fs::symlink_metadata(&stale_link).is_err(),
        "stale symlink should be deleted after interactive confirmation"
    );
}

#[test]
fn sync_prompts_for_stale_link_and_declines_no_on_stdin() {
    let temp = TempDir::new().expect("create temporary directory");
    let source_root = temp.path().join("sources");
    let config = temp.path().join("symbolic.json");
    let stale_src = source_root.join("removed.conf");
    let stale_link = temp.path().join("links/removed.conf");
    fs::create_dir_all(stale_link.parent().expect("stale link parent"))
        .expect("create link parent");
    fs::create_dir_all(&source_root).expect("create source root");
    unix_fs::symlink(&stale_src, &stale_link).expect("create stale symlink");
    write_config(&config, &[(&stale_link, &stale_src)]);

    symcfg()
        .current_dir(temp.path())
        .args([
            "sync",
            source_root.to_str().expect("utf-8 source path"),
            "--config",
            config.to_str().expect("utf-8 config path"),
        ])
        .write_stdin("n\n")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Source")
                .and(predicate::str::contains("Delete"))
                .and(predicate::str::contains(
                    "Sync complete      stale=1, removed=1, deleted=0, kept=1",
                )),
        );

    assert_no_entry(&config, &stale_link);
    assert_symlink_points_to(&stale_link, &stale_src);
}

#[test]
fn sync_delete_links_without_yes_fails_instead_of_ignoring_confirmation() {
    let temp = TempDir::new().expect("create temporary directory");
    let source_root = temp.path().join("sources");
    let config = temp.path().join("symbolic.json");
    let stale_src = source_root.join("removed.conf");
    let stale_link = temp.path().join("links/removed.conf");
    fs::create_dir_all(stale_link.parent().expect("stale link parent"))
        .expect("create link parent");
    fs::create_dir_all(&source_root).expect("create source root");
    unix_fs::symlink(&stale_src, &stale_link).expect("create stale symlink");
    write_config(&config, &[(&stale_link, &stale_src)]);

    symcfg()
        .current_dir(temp.path())
        .args([
            "sync",
            source_root.to_str().expect("utf-8 source path"),
            "--config",
            config.to_str().expect("utf-8 config path"),
            "--delete-links",
        ])
        .write_stdin("y\n")
        .assert()
        .failure()
        .stderr(predicate::str::contains("--delete-links").and(predicate::str::contains("--yes")));

    assert_has_entry(&config, &stale_link, &stale_src);
    assert_symlink_points_to(&stale_link, &stale_src);
}

#[test]
fn sync_keep_links_without_yes_fails_instead_of_ignoring_confirmation() {
    let temp = TempDir::new().expect("create temporary directory");
    let source_root = temp.path().join("sources");
    let config = temp.path().join("symbolic.json");
    let stale_src = source_root.join("removed.conf");
    let stale_link = temp.path().join("links/removed.conf");
    fs::create_dir_all(stale_link.parent().expect("stale link parent"))
        .expect("create link parent");
    fs::create_dir_all(&source_root).expect("create source root");
    unix_fs::symlink(&stale_src, &stale_link).expect("create stale symlink");
    write_config(&config, &[(&stale_link, &stale_src)]);

    symcfg()
        .current_dir(temp.path())
        .args([
            "sync",
            source_root.to_str().expect("utf-8 source path"),
            "--config",
            config.to_str().expect("utf-8 config path"),
            "--keep-links",
        ])
        .write_stdin("y\n")
        .assert()
        .failure()
        .stderr(predicate::str::contains("--keep-links").and(predicate::str::contains("--yes")));

    assert_has_entry(&config, &stale_link, &stale_src);
    assert_symlink_points_to(&stale_link, &stale_src);
}
