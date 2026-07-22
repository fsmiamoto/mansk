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

fn fixture_repo(root: &Path, contents: &str) -> (String, String) {
    let repo = root.join("upstream");
    fs::create_dir_all(repo.join("skills/review")).unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "Test"]);
    fs::write(repo.join("skills/review/SKILL.md"), contents).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "initial"]);
    let commit = git(&repo, &["rev-parse", "HEAD"]);
    (format!("file://{}", repo.display()), commit)
}

fn command(home: &Path, cache: &Path, manifest: &Path) -> Command {
    let mut command = Command::cargo_bin("mansk").unwrap();
    command
        .env("HOME", home)
        .env("XDG_CACHE_HOME", cache)
        .args(["--manifest", manifest.to_str().unwrap()]);
    command
}

fn advance_repo(source: &str, contents: &str) -> String {
    let repo = Path::new(source.strip_prefix("file://").unwrap());
    fs::write(repo.join("skills/review/SKILL.md"), contents).unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-m", contents]);
    git(repo, &["rev-parse", "HEAD"])
}

#[cfg(unix)]
#[test]
fn update_resolves_a_branch_records_the_commit_and_installs_the_skill() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, commit) = fixture_repo(temp.path(), "version one");
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\nsource = {source:?}\nselector = \"main\"\npath = \"skills/review\"\n"
        ),
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Link"));

    let lock: Value =
        serde_json::from_slice(&fs::read(temp.path().join("skills.lock")).unwrap()).unwrap();
    assert_eq!(lock["git"][&source], commit);
    let installed = home.join(".claude/skills/review");
    assert!(installed.is_symlink());
    assert_eq!(
        fs::read_to_string(installed.join("SKILL.md")).unwrap(),
        "version one"
    );
}

#[cfg(unix)]
#[test]
fn moved_branch_is_frozen_until_update() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, old_commit) = fixture_repo(temp.path(), "version one");
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\nsource = {source:?}\nselector = \"main\"\npath = \"skills/review\"\n"
        ),
    )
    .unwrap();
    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success();
    let installed = home.join(".claude/skills/review/SKILL.md");

    let new_commit = advance_repo(&source, "version two");
    command(&home, &cache, &manifest)
        .arg("sync")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Noop")
                .and(predicate::str::contains(new_commit[..7].to_owned()).not()),
        );
    assert_eq!(fs::read_to_string(&installed).unwrap(), "version one");

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "{} → {}",
            &old_commit[..7],
            &new_commit[..7]
        )));
    assert_eq!(fs::read_to_string(installed).unwrap(), "version two");
    let lock: Value =
        serde_json::from_slice(&fs::read(temp.path().join("skills.lock")).unwrap()).unwrap();
    assert_eq!(lock["git"][&source], new_commit);
}

#[cfg(unix)]
#[test]
fn cache_rebuild_uses_locked_commit() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, locked_commit) = fixture_repo(temp.path(), "locked version");
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\nsource = {source:?}\nselector = \"main\"\npath = \"skills/review\"\n"
        ),
    )
    .unwrap();
    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success();

    advance_repo(&source, "unlocked version");
    fs::remove_dir_all(cache.join("mansk/git")).unwrap();
    command(&home, &cache, &manifest)
        .arg("sync")
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(home.join(".claude/skills/review/SKILL.md")).unwrap(),
        "locked version"
    );
    let cache_head = git(
        fs::read_link(home.join(".claude/skills/review"))
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap(),
        &["rev-parse", "HEAD"],
    );
    assert_eq!(cache_head, locked_commit);
}

#[cfg(unix)]
#[test]
fn skills_at_the_same_commit_share_one_repository_checkout() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, _) = fixture_repo(temp.path(), "review");
    let repo = Path::new(source.strip_prefix("file://").unwrap());
    fs::create_dir_all(repo.join("skills/write")).unwrap();
    fs::write(repo.join("skills/write/SKILL.md"), "write").unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-m", "add write"]);
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\nsource = {source:?}\nselector = \"main\"\npath = \"skills/review\"\n[[skills]]\nsource = {source:?}\nselector = \"main\"\npath = \"skills/write\"\n"
        ),
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success();

    let review = fs::read_link(home.join(".claude/skills/review")).unwrap();
    let write = fs::read_link(home.join(".claude/skills/write")).unwrap();
    assert_eq!(
        review.parent().unwrap().parent(),
        write.parent().unwrap().parent()
    );
    assert_eq!(fs::read_dir(cache.join("mansk/git")).unwrap().count(), 1);
}

#[test]
fn selectors_at_different_commits_fail_before_target_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, _) = fixture_repo(temp.path(), "old");
    let repo = Path::new(source.strip_prefix("file://").unwrap());
    git(repo, &["tag", "old"]);
    advance_repo(&source, "new");
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\nsource = {source:?}\nselector = \"old\"\npath = \"skills/review\"\n[[skills]]\nsource = {source:?}\nselector = \"main\"\npath = \"skills/review\"\ntargets = [\"agents\"]\n"
        ),
    )
    .unwrap();
    let sentinel = home.join(".claude/skills/sentinel");
    fs::create_dir_all(sentinel.parent().unwrap()).unwrap();
    fs::write(&sentinel, "untouched").unwrap();

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("different commits"));

    assert_eq!(fs::read_to_string(sentinel).unwrap(), "untouched");
    assert!(!home.join(".agents").exists());
    assert!(!temp.path().join("skills.lock").exists());
}

#[test]
fn escaping_repository_path_fails_before_target_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, _) = fixture_repo(temp.path(), "review");
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\nsource = {source:?}\nselector = \"main\"\npath = \"../outside\"\n"
        ),
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("within the repository"));
    assert!(!home.join(".claude").exists());
    assert!(!temp.path().join("skills.lock").exists());
}

#[test]
fn missing_skill_document_in_git_fails_before_target_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, _) = fixture_repo(temp.path(), "review");
    let repo = Path::new(source.strip_prefix("file://").unwrap());
    fs::remove_file(repo.join("skills/review/SKILL.md")).unwrap();
    fs::write(repo.join("skills/review/README.md"), "not a skill").unwrap();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-m", "break skill"]);
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\nsource = {source:?}\nselector = \"main\"\npath = \"skills/review\"\n"
        ),
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("missing SKILL.md"));
    assert!(!home.join(".claude").exists());
    assert!(!temp.path().join("skills.lock").exists());
}

#[cfg(unix)]
#[test]
fn skill_document_symlink_outside_repository_fails_before_target_mutation() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, _) = fixture_repo(temp.path(), "review");
    let repo = Path::new(source.strip_prefix("file://").unwrap());
    let outside = temp.path().join("outside.md");
    fs::write(&outside, "not repository content").unwrap();
    fs::remove_file(repo.join("skills/review/SKILL.md")).unwrap();
    symlink(&outside, repo.join("skills/review/SKILL.md")).unwrap();
    git(repo, &["add", "-A"]);
    git(
        repo,
        &["commit", "-m", "replace skill document with symlink"],
    );
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\nsource = {source:?}\nselector = \"main\"\npath = \"skills/review\"\n"
        ),
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("regular SKILL.md"));
    assert!(!home.join(".claude").exists());
    assert!(!temp.path().join("skills.lock").exists());
}

#[test]
fn moved_commit_dry_run_preserves_lock_and_installed_content() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, old_commit) = fixture_repo(temp.path(), "old content");
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\nsource = {source:?}\nselector = \"main\"\npath = \"skills/review\"\n"
        ),
    )
    .unwrap();
    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success();
    let lock_path = temp.path().join("skills.lock");
    let lock_before = fs::read(&lock_path).unwrap();
    let installed = home.join(".claude/skills/review");
    let link_before = fs::read_link(&installed).unwrap();
    let content_before = fs::read(installed.join("SKILL.md")).unwrap();
    let new_commit = advance_repo(&source, "new content");

    command(&home, &cache, &manifest)
        .args(["update", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "{} → {}",
            &old_commit[..7],
            &new_commit[..7]
        )))
        .stdout(predicate::str::contains("Noop"));

    assert_eq!(fs::read(lock_path).unwrap(), lock_before);
    assert_eq!(fs::read_link(&installed).unwrap(), link_before);
    assert_eq!(
        fs::read(installed.join("SKILL.md")).unwrap(),
        content_before
    );
}

#[test]
fn git_command_failure_is_reported_before_target_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let source = format!(
        "file://{}",
        temp.path().join("no-such-repository").display()
    );
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\nsource = {source:?}\nselector = \"main\"\npath = \"skills/review\"\n"
        ),
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("git ls-remote").and(predicate::str::contains("failed")));
    assert!(!home.join(".claude").exists());
    assert!(!temp.path().join("skills.lock").exists());
}

#[test]
fn sync_rejects_a_non_commit_git_lock_value() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, _) = fixture_repo(temp.path(), "review");
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\nsource = {source:?}\nselector = \"main\"\npath = \"skills/review\"\n"
        ),
    )
    .unwrap();
    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success();
    let lock_path = temp.path().join("skills.lock");
    let mut lock: Value = serde_json::from_slice(&fs::read(&lock_path).unwrap()).unwrap();
    lock["git"][&source] = Value::String("main".into());
    fs::write(&lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
    let installed = home.join(".claude/skills/review/SKILL.md");

    command(&home, &cache, &manifest)
        .arg("sync")
        .assert()
        .failure()
        .stderr(predicate::str::contains("full commit"));
    assert_eq!(fs::read_to_string(installed).unwrap(), "review");
}

#[test]
fn missing_repository_path_fails_before_target_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, _) = fixture_repo(temp.path(), "review");
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\nsource = {source:?}\nselector = \"main\"\npath = \"skills/missing\"\n"
        ),
    )
    .unwrap();

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));
    assert!(!home.join(".claude").exists());
    assert!(!temp.path().join("skills.lock").exists());
}
