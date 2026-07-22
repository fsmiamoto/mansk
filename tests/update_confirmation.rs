use assert_cmd::Command;
use predicates::prelude::*;
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

fn collection_repo(root: &Path) -> (String, String) {
    let repo = root.join("upstream");
    fs::create_dir_all(repo.join("skills/alpha")).unwrap();
    fs::write(repo.join("skills/alpha/SKILL.md"), "alpha").unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "Test"]);
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

fn remove_collection_manifest(manifest: &Path) {
    fs::write(manifest, "schema = 1\ndefault-targets = [\"claude\"]\n").unwrap();
}

#[cfg(unix)]
#[test]
fn removing_a_collection_is_summarized_before_confirmation_and_decline_preserves_state() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, commit) = collection_repo(temp.path());
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[[collections]]\nsource = {source:?}\nselector = \"main\"\nroot = \"skills\"\n"
        ),
    )
    .unwrap();
    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success();
    let lock_path = temp.path().join("skills.lock");
    let lock_before = fs::read(&lock_path).unwrap();
    let installed = home.join(".claude/skills/alpha");
    let target_before = fs::read_link(&installed).unwrap();

    remove_collection_manifest(&manifest);

    command(&home, &cache, &manifest)
        .arg("update")
        .write_stdin("n\n")
        .assert()
        .success()
        .stdout(
            predicate::str::contains(format!("{source}: {} → (removed)", &commit[..7]))
                .and(predicate::str::contains(format!(
                    "{source}: members removed: alpha"
                )))
                .and(predicate::str::contains("Remove"))
                .and(predicate::str::contains("Declined")),
        )
        .stderr(predicate::str::contains("Apply update? [y/N]"));

    assert_eq!(fs::read(&lock_path).unwrap(), lock_before);
    assert_eq!(fs::read_link(installed).unwrap(), target_before);
}

#[cfg(unix)]
#[test]
fn empty_input_and_eof_decline_without_changing_lock_or_targets() {
    for input in [Some("\n"), None] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let cache = temp.path().join("cache");
        let (source, _) = collection_repo(temp.path());
        let manifest = temp.path().join("skills.toml");
        fs::write(
            &manifest,
            format!(
                "schema = 1\ndefault-targets = [\"claude\"]\n[[collections]]\nsource = {source:?}\nselector = \"main\"\nroot = \"skills\"\n"
            ),
        )
        .unwrap();
        command(&home, &cache, &manifest)
            .args(["update", "--yes"])
            .assert()
            .success();
        let lock_path = temp.path().join("skills.lock");
        let lock_before = fs::read(&lock_path).unwrap();
        let installed = home.join(".claude/skills/alpha");
        let target_before = fs::read_link(&installed).unwrap();
        remove_collection_manifest(&manifest);

        let mut update = command(&home, &cache, &manifest);
        update.arg("update");
        if let Some(input) = input {
            update.write_stdin(input);
        }
        update
            .assert()
            .success()
            .stdout(predicate::str::contains("Declined"))
            .stderr(predicate::str::contains("Apply update? [y/N]"));

        assert_eq!(fs::read(&lock_path).unwrap(), lock_before);
        assert_eq!(fs::read_link(&installed).unwrap(), target_before);
    }
}

#[cfg(unix)]
#[test]
fn positive_confirmation_applies_the_printed_update() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, commit) = collection_repo(temp.path());
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[[collections]]\nsource = {source:?}\nselector = \"main\"\nroot = \"skills\"\n"
        ),
    )
    .unwrap();
    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success();
    remove_collection_manifest(&manifest);

    command(&home, &cache, &manifest)
        .arg("update")
        .write_stdin("yes\n")
        .assert()
        .success()
        .stdout(
            predicate::str::contains(format!("{source}: {} → (removed)", &commit[..7]))
                .and(predicate::str::contains("members removed: alpha"))
                .and(predicate::str::contains("Remove")),
        )
        .stderr(predicate::str::contains("Apply update? [y/N]"));

    assert!(!home.join(".claude/skills/alpha").exists());
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(temp.path().join("skills.lock")).unwrap()).unwrap();
    assert_eq!(lock["collections"], serde_json::json!([]));
}

#[cfg(unix)]
#[test]
fn yes_flag_skips_the_prompt_but_keeps_the_change_summary() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let (source, commit) = collection_repo(temp.path());
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"claude\"]\n[[collections]]\nsource = {source:?}\nselector = \"main\"\nroot = \"skills\"\n"
        ),
    )
    .unwrap();
    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success();
    remove_collection_manifest(&manifest);

    command(&home, &cache, &manifest)
        .args(["update", "--yes"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains(format!("{source}: {} → (removed)", &commit[..7]))
                .and(predicate::str::contains("members removed: alpha")),
        )
        .stderr(predicate::str::contains("Apply update?").not());
}
