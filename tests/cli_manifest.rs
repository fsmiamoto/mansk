use assert_cmd::Command;
use predicates::prelude::*;
use std::{fs, path::Path, process::Command as GitCommand};

fn git(repo: &Path, args: &[&str]) {
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
}

#[test]
fn help_exposes_commands_and_global_manifest_option() {
    Command::cargo_bin("mansk")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("sync"))
        .stdout(predicate::str::contains("update"))
        .stdout(predicate::str::contains("--manifest"));

    Command::cargo_bin("mansk")
        .unwrap()
        .args(["sync", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--dry-run"))
        .stdout(predicate::str::contains("--yes").not());

    Command::cargo_bin("mansk")
        .unwrap()
        .args(["update", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--dry-run"))
        .stdout(predicate::str::contains("--yes"));
}

#[test]
fn manifest_defaults_to_xdg_config_and_explicit_path_takes_precedence() {
    let temp = tempfile::tempdir().unwrap();
    let config_manifest = temp.path().join("config/mansk/skills.toml");
    fs::create_dir_all(config_manifest.parent().unwrap()).unwrap();
    fs::write(&config_manifest, "not valid toml = [").unwrap();

    Command::cargo_bin("mansk")
        .unwrap()
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("HOME", temp.path().join("home"))
        .arg("sync")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            config_manifest.display().to_string(),
        ));

    let explicit = temp.path().join("explicit.toml");
    fs::write(&explicit, "schema = 1\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n").unwrap();
    Command::cargo_bin("mansk")
        .unwrap()
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("HOME", temp.path().join("home"))
        .args(["--manifest", explicit.to_str().unwrap(), "sync"])
        .assert()
        .success();
}

#[test]
fn settled_manifest_schema_accepts_git_local_and_collection_sources() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("upstream");
    fs::create_dir_all(repo.join("skills/review")).unwrap();
    fs::create_dir_all(repo.join("collection/collected")).unwrap();
    fs::write(repo.join("skills/review/SKILL.md"), "review").unwrap();
    fs::write(repo.join("collection/collected/SKILL.md"), "collected").unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "initial"]);
    let source = format!("file://{}", repo.display());
    let local = temp.path().join("local-skill");
    fs::create_dir_all(&local).unwrap();
    fs::write(local.join("SKILL.md"), "local").unwrap();
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        format!(
            r#"
schema = 1
default-targets = ["claude", "agents"]
[targets]
claude = ".claude/skills"
agents = ".agents/skills"

[[collections]]
source = {source:?}
selector = "main"
root = "collection"

[[skills]]
source = {source:?}
path = "skills/review"
selector = "main"
targets = ["claude"]

[[skills]]
path = "local-skill"
"#
        ),
    )
    .unwrap();

    Command::cargo_bin("mansk")
        .unwrap()
        .env("HOME", temp.path().join("home"))
        .env("XDG_CACHE_HOME", temp.path().join("cache"))
        .args(["--manifest", manifest.to_str().unwrap(), "update", "--yes"])
        .assert()
        .success();
}

#[test]
fn unsupported_schema_version_has_an_actionable_diagnostic() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("skills.toml");
    fs::write(&manifest, "schema = 2\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n").unwrap();

    Command::cargo_bin("mansk")
        .unwrap()
        .env("HOME", temp.path().join("home"))
        .args(["--manifest", manifest.to_str().unwrap(), "sync"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unsupported schema version 2"))
        .stderr(predicate::str::contains("expected 1"));
}

#[test]
fn unknown_manifest_fields_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        "schema = 1\n[[skills]]\npath = \"../local\"\ntarget = [\"claude\"]\n",
    )
    .unwrap();

    Command::cargo_bin("mansk")
        .unwrap()
        .env("HOME", temp.path().join("home"))
        .args(["--manifest", manifest.to_str().unwrap(), "sync"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown field `target`"));
}

#[test]
fn unknown_default_target_name_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("skills.toml");
    fs::write(&manifest, "schema = 1\ndefault-targets = [\"cursor\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n").unwrap();

    Command::cargo_bin("mansk")
        .unwrap()
        .env("HOME", temp.path().join("home"))
        .args(["--manifest", manifest.to_str().unwrap(), "sync"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown target `cursor`"))
        .stderr(predicate::str::contains("declare it under [targets]"));
}

#[test]
fn unknown_skill_target_name_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        "schema = 1\n[[skills]]\npath = \"../local\"\ntargets = [\"cursor\"]\n",
    )
    .unwrap();

    Command::cargo_bin("mansk")
        .unwrap()
        .env("HOME", temp.path().join("home"))
        .args(["--manifest", manifest.to_str().unwrap(), "sync"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown target `cursor`"));
}

#[test]
fn local_skill_cannot_have_a_git_selector() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        "schema = 1\n[[skills]]\npath = \"../local\"\nselector = \"main\"\n",
    )
    .unwrap();

    Command::cargo_bin("mansk")
        .unwrap()
        .env("HOME", temp.path().join("home"))
        .args(["--manifest", manifest.to_str().unwrap(), "sync"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "local skill must not specify `selector`",
        ));
}

#[test]
fn git_skill_requires_a_selector() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        "schema = 1\n[[skills]]\nsource = \"https://example.com/skill.git\"\npath = \"skill\"\n",
    )
    .unwrap();

    Command::cargo_bin("mansk")
        .unwrap()
        .env("HOME", temp.path().join("home"))
        .args(["--manifest", manifest.to_str().unwrap(), "sync"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Git skill must specify `selector`",
        ));
}

#[test]
fn invalid_manifest_creates_no_cache_or_target_directories() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache = temp.path().join("cache");
    let manifest = temp.path().join("invalid.toml");
    fs::write(&manifest, "schema = 2\n").unwrap();

    Command::cargo_bin("mansk")
        .unwrap()
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache)
        .args(["--manifest", manifest.to_str().unwrap(), "sync"])
        .assert()
        .failure();

    assert!(!cache.exists());
    assert!(!home.join(".claude").exists());
    assert!(!home.join(".agents").exists());
}
