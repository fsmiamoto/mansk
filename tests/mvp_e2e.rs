use assert_cmd::Command;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command as GitCommand,
};

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

fn commit(repo: &Path, message: &str) -> String {
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-m", message]);
    git(repo, &["rev-parse", "HEAD"])
}

fn command(home: &Path, config: &Path, cache: &Path) -> Command {
    let mut command = Command::cargo_bin("mansk").unwrap();
    command
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", config)
        .env("XDG_CACHE_HOME", cache);
    command
}

fn action_lines(output: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(output)
        .lines()
        .filter(|line| {
            line.starts_with("Link ") || line.starts_with("Remove ") || line.starts_with("Noop ")
        })
        .map(str::to_owned)
        .collect()
}

fn target_fingerprint(home: &Path) -> Vec<(PathBuf, String)> {
    let mut fingerprint = Vec::new();
    for relative in [".claude/skills", ".agents/skills"] {
        let target = home.join(relative);
        let Ok(entries) = fs::read_dir(&target) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            let value = if path.is_symlink() {
                format!("link:{}", fs::read_link(&path).unwrap().display())
            } else if path.is_file() {
                format!(
                    "file:{}",
                    String::from_utf8_lossy(&fs::read(&path).unwrap())
                )
            } else {
                "directory".to_owned()
            };
            fingerprint.push((path.strip_prefix(home).unwrap().to_owned(), value));
        }
    }
    fingerprint.sort();
    fingerprint
}

#[cfg(unix)]
#[test]
fn isolated_mixed_source_mvp_workflow_is_frozen_repeatable_and_safely_pruned() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let config = temp.path().join("config");
    let cache = temp.path().join("cache");
    let repo = temp.path().join("upstream");
    fs::create_dir_all(repo.join("explicit/review")).unwrap();
    fs::create_dir_all(repo.join("collection/alpha")).unwrap();
    fs::create_dir_all(repo.join("collection/removed")).unwrap();
    fs::write(repo.join("explicit/review/SKILL.md"), "review one").unwrap();
    fs::write(repo.join("collection/alpha/SKILL.md"), "alpha one").unwrap();
    fs::write(repo.join("collection/removed/SKILL.md"), "removed one").unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "Test"]);
    let first_commit = commit(&repo, "initial");
    let source = format!("file://{}", repo.display());

    let local = temp.path().join("local-note");
    fs::create_dir_all(&local).unwrap();
    fs::write(local.join("SKILL.md"), "local").unwrap();
    let manifest = config.join("mansk/skills.toml");
    fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"agents\"]\n[[collections]]\nsource = {source:?}\nselector = \"main\"\nroot = \"collection\"\n[[skills]]\nsource = {source:?}\nselector = \"main\"\npath = \"explicit/review\"\n[[skills]]\npath = \"../../local-note\"\ntargets = [\"claude\"]\n"
        ),
    )
    .unwrap();

    command(&home, &config, &cache)
        .args(["update", "--yes"])
        .assert()
        .success();
    let lock_path = config.join("mansk/skills.lock");
    assert!(
        fs::read_to_string(&lock_path)
            .unwrap()
            .contains(&first_commit)
    );
    assert_eq!(
        fs::read_to_string(home.join(".agents/skills/review/SKILL.md")).unwrap(),
        "review one"
    );
    assert!(home.join(".claude/skills/local-note").is_symlink());

    for _ in 0..2 {
        let output = command(&home, &config, &cache)
            .arg("sync")
            .output()
            .unwrap();
        assert!(output.status.success());
        let actions = action_lines(&output.stdout);
        assert_eq!(actions.len(), 4);
        assert!(actions.iter().all(|line| line.starts_with("Noop ")));
    }

    fs::write(repo.join("explicit/review/SKILL.md"), "review two").unwrap();
    fs::write(repo.join("collection/alpha/SKILL.md"), "alpha two").unwrap();
    fs::remove_dir_all(repo.join("collection/removed")).unwrap();
    fs::create_dir_all(repo.join("collection/beta")).unwrap();
    fs::write(repo.join("collection/beta/SKILL.md"), "beta two").unwrap();
    let second_commit = commit(&repo, "advance branch");

    command(&home, &config, &cache)
        .arg("sync")
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(home.join(".agents/skills/review/SKILL.md")).unwrap(),
        "review one"
    );
    assert!(home.join(".agents/skills/removed").is_symlink());
    assert!(!home.join(".agents/skills/beta").exists());

    let lock_before_dry_run = fs::read(&lock_path).unwrap();
    let targets_before_dry_run = target_fingerprint(&home);
    let dry_update = command(&home, &config, &cache)
        .args(["update", "--dry-run"])
        .output()
        .unwrap();
    assert!(dry_update.status.success());
    assert_eq!(fs::read(&lock_path).unwrap(), lock_before_dry_run);
    assert_eq!(target_fingerprint(&home), targets_before_dry_run);

    let applied_update = command(&home, &config, &cache)
        .args(["update", "--yes"])
        .output()
        .unwrap();
    assert!(applied_update.status.success());
    assert_eq!(
        action_lines(&dry_update.stdout),
        action_lines(&applied_update.stdout)
    );
    let summary = String::from_utf8_lossy(&applied_update.stdout);
    assert!(summary.contains(&format!("{} → {}", &first_commit[..7], &second_commit[..7])));
    assert!(summary.contains("members added: beta"));
    assert!(summary.contains("members removed: removed"));
    assert_eq!(
        fs::read_to_string(home.join(".agents/skills/review/SKILL.md")).unwrap(),
        "review two"
    );
    assert_eq!(
        fs::read_to_string(home.join(".agents/skills/alpha/SKILL.md")).unwrap(),
        "alpha two"
    );
    assert!(home.join(".agents/skills/beta").is_symlink());
    assert!(!home.join(".agents/skills/removed").exists());

    let unmanaged = home.join(".agents/skills/unmanaged");
    fs::write(&unmanaged, "keep me").unwrap();
    fs::write(
        &manifest,
        format!(
            "schema = 1\ndefault-targets = [\"agents\"]\n[[collections]]\nsource = {source:?}\nselector = \"main\"\nroot = \"collection\"\n"
        ),
    )
    .unwrap();
    let targets_before_sync_dry_run = target_fingerprint(&home);
    let dry_sync = command(&home, &config, &cache)
        .args(["sync", "--dry-run"])
        .output()
        .unwrap();
    assert!(dry_sync.status.success());
    assert_eq!(target_fingerprint(&home), targets_before_sync_dry_run);

    let applied_sync = command(&home, &config, &cache)
        .arg("sync")
        .output()
        .unwrap();
    assert!(applied_sync.status.success());
    assert_eq!(
        action_lines(&dry_sync.stdout),
        action_lines(&applied_sync.stdout)
    );
    assert!(!home.join(".agents/skills/review").exists());
    assert!(!home.join(".claude/skills/local-note").exists());
    assert!(home.join(".agents/skills/alpha").is_symlink());
    assert!(home.join(".agents/skills/beta").is_symlink());
    assert_eq!(fs::read_to_string(unmanaged).unwrap(), "keep me");
}
