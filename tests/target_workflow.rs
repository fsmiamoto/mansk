use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

fn command(home: &std::path::Path, cache: &std::path::Path, manifest: &std::path::Path) -> Command {
    let mut command = Command::cargo_bin("mansk").unwrap();
    command
        .env("HOME", home)
        .env("XDG_CACHE_HOME", cache)
        .args(["--manifest", manifest.to_str().unwrap()]);
    command
}

fn skill(path: &std::path::Path, contents: &str) {
    fs::create_dir_all(path).unwrap();
    fs::write(path.join("SKILL.md"), contents).unwrap();
}

#[test]
fn arbitrary_relative_and_absolute_targets_support_defaults_fanout_and_overrides() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let absolute = temp.path().join("absolute-skills");
    let manifest = temp.path().join("skills.toml");
    skill(&temp.path().join("defaulted"), "defaulted");
    skill(&temp.path().join("overridden"), "overridden");
    fs::write(
        &manifest,
        format!(
            r#"schema = 1
default-targets = ["editor", "shared"]

[targets]
editor = ".editor/skills"
shared = {absolute:?}
alternate = ".alternate/skills"

[[skills]]
path = "defaulted"

[[skills]]
path = "overridden"
targets = ["alternate"]
"#,
        ),
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success();

    assert!(home.join(".editor/skills/defaulted").is_symlink());
    assert!(absolute.join("defaulted").is_symlink());
    assert!(home.join(".alternate/skills/overridden").is_symlink());
    assert!(!home.join(".editor/skills/overridden").exists());
    assert!(!absolute.join("overridden").exists());
    assert!(!home.join(".alternate/skills/defaulted").exists());
}

#[test]
fn absolute_targets_do_not_require_home() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("unused-home");
    let cache = temp.path().join("cache");
    let absolute = temp.path().join("absolute-skills");
    let manifest = temp.path().join("skills.toml");
    skill(&temp.path().join("review"), "review");
    fs::write(
        &manifest,
        format!(
            r#"schema = 1
default-targets = ["absolute"]

[targets]
absolute = {absolute:?}

[[skills]]
path = "review"
"#,
        ),
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .env_remove("HOME")
        .args(["update", "--yes"])
        .assert()
        .success();

    assert!(absolute.join("review").is_symlink());
}

#[test]
fn empty_target_paths_are_rejected_even_when_the_manifest_has_no_skills() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        r#"schema = 1
default-targets = ["empty"]

[targets]
empty = ""
"#,
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("target `empty`").and(predicate::str::contains("empty")));

    assert!(!cache.exists());
    assert!(!temp.path().join("skills.lock").exists());
}

#[test]
fn lexically_equivalent_target_destinations_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let manifest = temp.path().join("skills.toml");
    skill(&temp.path().join("review"), "review");
    fs::write(
        &manifest,
        r#"schema = 1
default-targets = ["first"]

[targets]
first = ".shared/skills"
second = ".other/../.shared/skills"

[[skills]]
path = "review"
"#,
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("first")
                .and(predicate::str::contains("second"))
                .and(predicate::str::contains("same directory")),
        );

    assert!(!home.join(".shared").exists());
    assert!(!temp.path().join("skills.lock").exists());
}

#[test]
fn existing_target_aliases_to_the_same_directory_are_rejected() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let shared = temp.path().join("shared");
    let alias = temp.path().join("alias");
    let manifest = temp.path().join("skills.toml");
    fs::create_dir(&shared).unwrap();
    symlink(&shared, &alias).unwrap();
    fs::write(
        &manifest,
        format!(
            r#"schema = 1

[targets]
first = {shared:?}
second = {alias:?}
"#,
        ),
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .arg("sync")
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("first")
                .and(predicate::str::contains("second"))
                .and(predicate::str::contains("same directory")),
        );
}

#[test]
fn target_directories_inside_the_mansk_cache_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let manifest = temp.path().join("skills.toml");
    skill(&temp.path().join("review"), "review");
    fs::write(
        &manifest,
        r#"schema = 1
default-targets = ["cached"]

[targets]
cached = "../cache/mansk/../mansk/installed"

[[skills]]
path = "review"
"#,
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("target `cached`").and(predicate::str::contains("cache")));

    assert!(!cache.exists());
    assert!(!temp.path().join("skills.lock").exists());
}

#[test]
fn dangling_target_symlinks_are_rejected_before_cache_creation() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let alias = temp.path().join("cache-alias");
    let manifest = temp.path().join("skills.toml");
    symlink(cache.join("mansk"), &alias).unwrap();
    skill(&temp.path().join("review"), "review");
    fs::write(
        &manifest,
        format!(
            r#"schema = 1
default-targets = ["dangling"]

[targets]
dangling = {target:?}

[[skills]]
path = "review"
"#,
            target = alias.join("installed"),
        ),
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("target `dangling`").and(predicate::str::contains("symlink")),
        );

    assert!(!cache.exists());
    assert!(!temp.path().join("skills.lock").exists());
}

#[test]
fn declared_targets_are_scanned_and_pruned_even_when_unused() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let manifest = temp.path().join("skills.toml");
    skill(&temp.path().join("review"), "review");
    fs::write(
        &manifest,
        r#"schema = 1
default-targets = ["kept"]

[targets]
kept = ".kept/skills"

[[skills]]
path = "review"
"#,
    )
    .unwrap();
    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success();
    let installed = home.join(".kept/skills/review");
    assert!(installed.is_symlink());

    fs::write(
        &manifest,
        r#"schema = 1

[targets]
kept = ".kept/skills"
"#,
    )
    .unwrap();
    fs::remove_file(temp.path().join("skills.lock")).unwrap();
    command(&home, &cache, &manifest)
        .arg("sync")
        .assert()
        .success();

    assert!(!installed.exists());
}

#[test]
fn target_path_changes_and_removals_leave_old_links_because_targets_are_stateless() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let manifest = temp.path().join("skills.toml");
    skill(&temp.path().join("review"), "review");
    fs::write(
        &manifest,
        r#"schema = 1
default-targets = ["place"]

[targets]
place = ".original/skills"

[[skills]]
path = "review"
"#,
    )
    .unwrap();
    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success();
    let original = home.join(".original/skills/review");
    assert!(original.is_symlink());

    fs::write(
        &manifest,
        r#"schema = 1
default-targets = ["place"]

[targets]
place = ".moved/skills"

[[skills]]
path = "review"
"#,
    )
    .unwrap();
    command(&home, &cache, &manifest)
        .arg("sync")
        .assert()
        .success();
    let moved = home.join(".moved/skills/review");
    assert!(original.is_symlink());
    assert!(moved.is_symlink());

    fs::write(
        &manifest,
        r#"schema = 1

[[skills]]
path = "review"
"#,
    )
    .unwrap();
    command(&home, &cache, &manifest)
        .arg("sync")
        .assert()
        .success();

    assert!(original.is_symlink());
    assert!(moved.is_symlink());
}
