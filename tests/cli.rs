use std::{fs, os::unix::fs as unix_fs, path::Path};

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
    let config = read_config(config_path);
    let expected_link = link.to_string_lossy();
    let expected_src = src.to_string_lossy();

    assert!(
        links(&config).iter().any(|entry| {
            entry["link"].as_str() == Some(expected_link.as_ref())
                && entry["src"].as_str() == Some(expected_src.as_ref())
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
    assert_eq!(actual, src);
}

fn assert_symlink_exists(path: &Path) {
    let metadata = fs::symlink_metadata(path).expect("symlink metadata");
    assert!(
        metadata.file_type().is_symlink(),
        "{path:?} should be a symlink"
    );
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
            "--in",
            link_root.to_str().expect("utf-8 link path"),
        ])
        .assert()
        .success();

    let config = temp.path().join(DEFAULT_CONFIG);
    assert!(config.exists(), "search should create symbolic.json in cwd");
    assert_has_entry(&config, &link, &src);
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
            "--in",
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
    assert_has_entry(&custom_config, &link, &src);
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
        .success();

    assert_symlink_points_to(&link, &src);
    assert_has_entry(&config, &link, &src);
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
    assert_has_entry(&config, &link, &src);
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
        .stdout(
            predicate::str::contains("created")
                .and(predicate::str::contains("skipped"))
                .and(predicate::str::contains("conflict")),
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
        .stdout(predicate::str::contains(
            "Apply complete: created=0, skipped=1, conflict=0",
        ));

    assert!(!link.parent().expect("link parent").exists());
    assert!(!link.exists());
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
            "--config",
            config.to_str().expect("utf-8 config path"),
            "--source",
            source_root.to_str().expect("utf-8 source path"),
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
            "--config",
            config.to_str().expect("utf-8 config path"),
            "--source",
            source_root.to_str().expect("utf-8 source path"),
            "--yes",
            "--keep-links",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("removed").and(predicate::str::contains("kept")));

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
            "--config",
            config.to_str().expect("utf-8 config path"),
            "--source",
            source_root.to_str().expect("utf-8 source path"),
            "--yes",
            "--delete-links",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("removed").and(predicate::str::contains("deleted")));

    assert_no_entry(&config, &matching_link);
    assert_no_entry(&config, &nonmatching_link);
    assert!(
        fs::symlink_metadata(&matching_link).is_err(),
        "matching stale symlink should be deleted"
    );
    assert_symlink_points_to(&nonmatching_link, &unrelated_target);
}

#[test]
fn validate_prints_english_success_for_valid_config_and_failure_for_invalid_config() {
    let temp = TempDir::new().expect("create temporary directory");
    let valid_config = temp.path().join("valid.json");
    let invalid_config = temp.path().join("invalid.json");
    let src = temp.path().join("sources/vimrc");
    let link = temp.path().join("links/vimrc");
    write_file(&src, "syntax on\n");
    write_config(&valid_config, &[(&link, &src)]);
    fs::write(
        &invalid_config,
        serde_json::to_string_pretty(&json!({
            "version": 1,
            "links": [{ "link": link, "target": src }]
        }))
        .expect("serialize invalid config"),
    )
    .expect("write invalid config");

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
            invalid_config.to_str().expect("utf-8 config path"),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid").or(predicate::str::contains("error")));
}
