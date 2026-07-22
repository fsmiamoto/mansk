use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::{fs, path::Path, process::Command as GitCommand};

fn git(repo: &Path, args: &[&str]) -> String {
    let output = GitCommand::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn init_repo(root: &Path) -> (String, String) {
    let repo = root.join("upstream");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "Test"]);
    (
        format!("file://{}", repo.display()),
        repo.display().to_string(),
    )
}

fn commit(repo: &Path, message: &str) -> String {
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-m", message]);
    git(repo, &["rev-parse", "HEAD"])
}

fn command(home: &Path, cache: &Path, manifest: &Path) -> Command {
    let mut command = Command::cargo_bin("mansk").unwrap();
    command
        .env("HOME", home)
        .env("XDG_CACHE_HOME", cache)
        .args(["--manifest", manifest.to_str().unwrap()]);
    command
}

#[test]
fn duplicate_collection_and_local_names_fail_before_target_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, repo_path) = init_repo(temp.path());
    let repo = Path::new(&repo_path);
    fs::create_dir_all(repo.join("skills/review")).unwrap();
    fs::write(repo.join("skills/review/SKILL.md"), "Git review").unwrap();
    commit(repo, "initial");
    fs::create_dir_all(temp.path().join("local/review")).unwrap();
    fs::write(temp.path().join("local/review/SKILL.md"), "local review").unwrap();
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n[[collections]]\nsource = {source:?}\nselector = \"main\"\nroot = \"skills\"\n[[skills]]\npath = \"local/review\"\n"
        ),
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("duplicate skill name `review`"));
    assert!(!home.join(".claude").exists());
    assert!(!temp.path().join("skills.lock").exists());
}

#[test]
fn duplicate_collection_and_explicit_git_names_fail_before_target_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, repo_path) = init_repo(temp.path());
    let repo = Path::new(&repo_path);
    fs::create_dir_all(repo.join("skills/review")).unwrap();
    fs::write(repo.join("skills/review/SKILL.md"), "review").unwrap();
    commit(repo, "initial");
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n[[collections]]\nsource = {source:?}\nselector = \"main\"\nroot = \"skills\"\n[[skills]]\nsource = {source:?}\nselector = \"main\"\npath = \"skills/review\"\n"
        ),
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("duplicate skill name `review`"));
    assert!(!home.join(".claude").exists());
    assert!(!temp.path().join("skills.lock").exists());
}

#[test]
fn sync_rejects_a_lock_for_a_different_collection_root() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, repo_path) = init_repo(temp.path());
    let repo = Path::new(&repo_path);
    for root in ["first", "second"] {
        fs::create_dir_all(repo.join(root).join("review")).unwrap();
        fs::write(repo.join(root).join("review/SKILL.md"), root).unwrap();
    }
    commit(repo, "initial");
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n[[collections]]\nsource = {source:?}\nselector = \"main\"\nroot = \"first\"\n"
        ),
    )
    .unwrap();
    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success();
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n[[collections]]\nsource = {source:?}\nselector = \"main\"\nroot = \"second\"\n"
        ),
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .arg("sync")
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "skills.lock does not cover collection",
        ));
    assert_eq!(
        fs::read_to_string(home.join(".claude/skills/review/SKILL.md")).unwrap(),
        "first"
    );
}

#[test]
fn sync_rejects_a_lock_that_does_not_cover_the_collection() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, repo_path) = init_repo(temp.path());
    let repo = Path::new(&repo_path);
    fs::create_dir_all(repo.join("review")).unwrap();
    fs::write(repo.join("review/SKILL.md"), "review").unwrap();
    commit(repo, "initial");
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n[[collections]]\nsource = {source:?}\nselector = \"main\"\n"
        ),
    )
    .unwrap();
    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success();
    let lock_path = temp.path().join("skills.lock");
    let mut lock: Value = serde_json::from_slice(&fs::read(&lock_path).unwrap()).unwrap();
    lock["collections"] = serde_json::json!([]);
    fs::write(&lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();

    command(&home, &cache, &manifest)
        .arg("sync")
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "skills.lock does not cover collections",
        ));
    assert!(home.join(".claude/skills/review").is_symlink());
}

#[test]
fn missing_collection_root_fails_before_target_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, repo_path) = init_repo(temp.path());
    let repo = Path::new(&repo_path);
    fs::write(repo.join("README.md"), "empty").unwrap();
    commit(repo, "initial");
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n[[collections]]\nsource = {source:?}\nselector = \"main\"\nroot = \"missing\"\n"
        ),
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .failure()
        .stderr(
            predicates::str::contains("collection root missing")
                .and(predicates::str::contains("does not exist")),
        );
    assert!(!home.join(".claude").exists());
    assert!(!temp.path().join("skills.lock").exists());
}

#[cfg(unix)]
#[test]
fn collection_membership_is_frozen_until_update() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, repo_path) = init_repo(temp.path());
    let repo = Path::new(&repo_path);
    for (name, content) in [("alpha", "old alpha"), ("removed", "old removed")] {
        fs::create_dir_all(repo.join("skills").join(name)).unwrap();
        fs::write(repo.join("skills").join(name).join("SKILL.md"), content).unwrap();
    }
    commit(repo, "initial");
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\", \"agents\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n[[collections]]\nsource = {source:?}\nselector = \"main\"\nroot = \"skills\"\n"
        ),
    )
    .unwrap();
    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success();

    fs::write(repo.join("skills/alpha/SKILL.md"), "new alpha").unwrap();
    fs::remove_dir_all(repo.join("skills/removed")).unwrap();
    fs::create_dir_all(repo.join("skills/added")).unwrap();
    fs::write(repo.join("skills/added/SKILL.md"), "new added").unwrap();
    commit(repo, "change membership");

    command(&home, &cache, &manifest)
        .arg("sync")
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(home.join(".claude/skills/alpha/SKILL.md")).unwrap(),
        "old alpha"
    );
    assert!(home.join(".claude/skills/removed").is_symlink());
    assert!(!home.join(".claude/skills/added").exists());

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success()
        .stdout(predicates::str::contains("members added: added"))
        .stdout(predicates::str::contains("members removed: removed"));

    for target in [".claude/skills", ".agents/skills"] {
        assert!(home.join(target).join("added").is_symlink());
        assert!(!home.join(target).join("removed").exists());
    }
    assert_eq!(
        fs::read_to_string(home.join(".claude/skills/alpha/SKILL.md")).unwrap(),
        "new alpha"
    );
    let lock: Value =
        serde_json::from_slice(&fs::read(temp.path().join("skills.lock")).unwrap()).unwrap();
    assert_eq!(
        lock["collections"][0]["members"],
        serde_json::json!(["added", "alpha"])
    );
}

#[cfg(unix)]
#[test]
fn collection_discovery_is_shallow_under_configured_root() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, repo_path) = init_repo(temp.path());
    let repo = Path::new(&repo_path);
    fs::create_dir_all(repo.join("skills/direct")).unwrap();
    fs::write(repo.join("skills/direct/SKILL.md"), "direct").unwrap();
    fs::create_dir_all(repo.join("skills/not-a-skill/nested")).unwrap();
    fs::write(repo.join("skills/not-a-skill/nested/SKILL.md"), "nested").unwrap();
    fs::create_dir_all(repo.join("sibling")).unwrap();
    fs::write(repo.join("sibling/SKILL.md"), "sibling").unwrap();
    commit(repo, "initial");
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n[[collections]]\nsource = {source:?}\nselector = \"main\"\nroot = \"skills\"\n"
        ),
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success();

    assert!(home.join(".claude/skills/direct").is_symlink());
    assert!(!home.join(".claude/skills/not-a-skill").exists());
    assert!(!home.join(".claude/skills/nested").exists());
    assert!(!home.join(".claude/skills/sibling").exists());
    let lock: Value =
        serde_json::from_slice(&fs::read(temp.path().join("skills.lock")).unwrap()).unwrap();
    assert_eq!(
        lock["collections"][0]["members"],
        serde_json::json!(["direct"])
    );
}

#[cfg(unix)]
#[test]
fn collection_with_omitted_root_installs_root_members_and_locks_them() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, repo_path) = init_repo(temp.path());
    let repo = Path::new(&repo_path);
    fs::create_dir_all(repo.join("review")).unwrap();
    fs::write(repo.join("review/SKILL.md"), "review").unwrap();
    let commit_id = commit(repo, "initial");
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n[[collections]]\nsource = {source:?}\nselector = \"main\"\n"
        ),
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(home.join(".claude/skills/review/SKILL.md")).unwrap(),
        "review"
    );
    let lock: Value =
        serde_json::from_slice(&fs::read(temp.path().join("skills.lock")).unwrap()).unwrap();
    assert_eq!(lock["collections"][0]["source"], source);
    assert_eq!(lock["collections"][0]["commit"], commit_id);
    assert_eq!(
        lock["collections"][0]["members"],
        serde_json::json!(["review"])
    );
    assert!(lock["collections"][0].get("hashes").is_none());
}
