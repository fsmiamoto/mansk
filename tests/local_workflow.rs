use assert_cmd::Command;
use serde_json::Value;
use std::fs;

#[cfg(unix)]
#[test]
fn local_update_sync_noop_and_prune() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("isolated-home");
    let config_home = temp.path().join("isolated-config");
    let cache_home = temp.path().join("isolated-cache");
    for name in ["keep", "remove"] {
        let skill = temp.path().join(name);
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), name).unwrap();
    }
    let manifest = config_home.join("mansk/skills.toml");
    fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\npath = \"../../keep\"\n[[skills]]\npath = \"../../remove\"\n",
    )
    .unwrap();
    let command = || {
        let mut command = Command::cargo_bin("mansk").unwrap();
        command
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", &config_home)
            .env("XDG_CACHE_HOME", &cache_home);
        command
    };

    command().args(["update", "--yes"]).assert().success();
    for _ in 0..2 {
        command()
            .arg("sync")
            .assert()
            .success()
            .stdout(predicates::str::contains("Noop"));
    }

    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\npath = \"../../keep\"\n",
    )
    .unwrap();
    command()
        .arg("sync")
        .assert()
        .success()
        .stdout(predicates::str::contains("Remove"));

    let target = home.join(".claude/skills");
    let mut entries: Vec<_> = fs::read_dir(&target)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    entries.sort();
    assert_eq!(entries, vec![std::ffi::OsString::from("keep")]);
    assert_eq!(
        fs::read_link(target.join("keep")).unwrap(),
        cache_home.join("mansk/local/keep")
    );
    assert!(!target.join("remove").exists());
}

#[cfg(unix)]
#[test]
fn update_installs_a_local_skill_and_records_its_source() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache_home = temp.path().join("cache");
    let skill = temp.path().join("skills/review");
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "review instructions").unwrap();
    let manifest = temp.path().join("config/skills.toml");
    fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\", \"agents\"]\n[[skills]]\npath = \"../skills/review\"\n",
    )
    .unwrap();

    Command::cargo_bin("mansk")
        .unwrap()
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache_home)
        .args(["--manifest", manifest.to_str().unwrap(), "update", "--yes"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Link"));

    let lock_text = fs::read_to_string(temp.path().join("config/skills.lock")).unwrap();
    let lock: Value = serde_json::from_str(&lock_text).unwrap();
    assert_eq!(lock["local"][0]["source"], "../skills/review");
    assert!(!lock_text.to_ascii_lowercase().contains("hash"));

    let cached = cache_home.join("mansk/local/review");
    assert_eq!(
        fs::read_to_string(cached.join("SKILL.md")).unwrap(),
        "review instructions"
    );
    assert_eq!(
        fs::read_link(home.join(".claude/skills/review")).unwrap(),
        cached
    );
    assert_eq!(
        fs::read_link(home.join(".agents/skills/review")).unwrap(),
        cached
    );
}

#[cfg(unix)]
#[test]
fn duplicate_effective_targets_install_each_skill_once() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache_home = temp.path().join("cache");
    let skill = temp.path().join("review");
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "review instructions").unwrap();
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\", \"claude\"]\n[[skills]]\npath = \"review\"\n",
    )
    .unwrap();

    Command::cargo_bin("mansk")
        .unwrap()
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache_home)
        .args(["--manifest", manifest.to_str().unwrap(), "update", "--yes"])
        .assert()
        .success();

    assert!(home.join(".claude/skills/review").is_symlink());
}

#[cfg(unix)]
#[test]
fn sync_refreshes_local_content_and_reports_an_existing_link_as_noop() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache_home = temp.path().join("cache");
    let skill = temp.path().join("review");
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "version one").unwrap();
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\npath = \"review\"\n",
    )
    .unwrap();

    Command::cargo_bin("mansk")
        .unwrap()
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache_home)
        .args(["--manifest", manifest.to_str().unwrap(), "update", "--yes"])
        .assert()
        .success();

    fs::write(skill.join("SKILL.md"), "version two").unwrap();
    Command::cargo_bin("mansk")
        .unwrap()
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache_home)
        .args(["--manifest", manifest.to_str().unwrap(), "sync"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Noop"));

    let installed = home.join(".claude/skills/review/SKILL.md");
    assert_eq!(fs::read_to_string(&installed).unwrap(), "version two");

    Command::cargo_bin("mansk")
        .unwrap()
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache_home)
        .args(["--manifest", manifest.to_str().unwrap(), "sync"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Noop"));
    assert_eq!(fs::read_to_string(installed).unwrap(), "version two");
}

#[cfg(unix)]
#[test]
fn per_skill_targets_override_manifest_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache_home = temp.path().join("cache");
    for name in ["defaulted", "overridden"] {
        let skill = temp.path().join(name);
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), name).unwrap();
    }
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        r#"schema = 1
default-targets = ["claude"]
[[skills]]
path = "defaulted"
[[skills]]
path = "overridden"
targets = ["agents"]
"#,
    )
    .unwrap();

    Command::cargo_bin("mansk")
        .unwrap()
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache_home)
        .args(["--manifest", manifest.to_str().unwrap(), "update", "--yes"])
        .assert()
        .success();

    assert!(home.join(".claude/skills/defaulted").is_symlink());
    assert!(!home.join(".agents/skills/defaulted").exists());
    assert!(home.join(".agents/skills/overridden").is_symlink());
    assert!(!home.join(".claude/skills/overridden").exists());
}

#[test]
fn sync_requires_a_lock_covering_every_local_source_before_creating_targets() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache_home = temp.path().join("cache");
    let skill = temp.path().join("review");
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "review").unwrap();
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\npath = \"review\"\n",
    )
    .unwrap();

    Command::cargo_bin("mansk")
        .unwrap()
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache_home)
        .args(["--manifest", manifest.to_str().unwrap(), "sync"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("skills.lock"));
    assert!(!home.join(".claude").exists());

    fs::write(
        temp.path().join("skills.lock"),
        "{\"schema\":1,\"local\":[]}",
    )
    .unwrap();
    Command::cargo_bin("mansk")
        .unwrap()
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache_home)
        .args(["--manifest", manifest.to_str().unwrap(), "sync"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "does not cover local source `review`",
        ));
    assert!(!home.join(".claude").exists());
}

#[test]
fn missing_skill_document_fails_before_any_target_directory_is_mutated() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache_home = temp.path().join("cache");
    fs::create_dir_all(temp.path().join("broken")).unwrap();
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\", \"agents\"]\n[[skills]]\npath = \"broken\"\n",
    )
    .unwrap();

    Command::cargo_bin("mansk")
        .unwrap()
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache_home)
        .args(["--manifest", manifest.to_str().unwrap(), "update", "--yes"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("missing SKILL.md"));

    assert!(!home.join(".claude").exists());
    assert!(!home.join(".agents").exists());
    assert!(!temp.path().join("skills.lock").exists());
}

#[cfg(unix)]
#[test]
fn declining_an_update_does_not_refresh_content_visible_through_installed_links() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache_home = temp.path().join("cache");
    let skill = temp.path().join("review");
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "installed version").unwrap();
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\npath = \"review\"\n",
    )
    .unwrap();

    let command = || {
        let mut command = Command::cargo_bin("mansk").unwrap();
        command
            .env("HOME", &home)
            .env("XDG_CACHE_HOME", &cache_home)
            .args(["--manifest", manifest.to_str().unwrap()]);
        command
    };
    command().args(["update", "--yes"]).assert().success();
    let installed_document = home.join(".claude/skills/review/SKILL.md");
    let lock_before = fs::read(temp.path().join("skills.lock")).unwrap();

    fs::write(skill.join("SKILL.md"), "declined version").unwrap();
    command()
        .arg("update")
        .write_stdin("n\n")
        .assert()
        .success()
        .stdout(predicates::str::contains("Declined"));

    assert_eq!(
        fs::read_to_string(installed_document).unwrap(),
        "installed version"
    );
    assert_eq!(
        fs::read_to_string(cache_home.join("mansk/local/review/SKILL.md")).unwrap(),
        "installed version"
    );
    assert_eq!(
        fs::read(temp.path().join("skills.lock")).unwrap(),
        lock_before
    );
}

#[cfg(unix)]
#[test]
fn cache_staging_does_not_clobber_a_skill_with_the_old_temporary_name() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache_home = temp.path().join("cache");
    for (name, contents) in [(".review.tmp", "hidden skill"), ("review", "regular skill")] {
        let skill = temp.path().join(name);
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), contents).unwrap();
    }
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\npath = \".review.tmp\"\n[[skills]]\npath = \"review\"\n",
    )
    .unwrap();

    Command::cargo_bin("mansk")
        .unwrap()
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache_home)
        .args(["--manifest", manifest.to_str().unwrap(), "update", "--yes"])
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(home.join(".claude/skills/.review.tmp/SKILL.md")).unwrap(),
        "hidden skill"
    );
    assert_eq!(
        fs::read_to_string(home.join(".claude/skills/review/SKILL.md")).unwrap(),
        "regular skill"
    );
}

#[cfg(unix)]
#[test]
fn dry_runs_print_plans_without_writing_lock_or_target_entries() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache_home = temp.path().join("cache");
    for name in ["one", "two"] {
        fs::create_dir_all(temp.path().join(name)).unwrap();
        fs::write(temp.path().join(name).join("SKILL.md"), name).unwrap();
    }
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\npath = \"one\"\n",
    )
    .unwrap();
    let command = || {
        let mut command = Command::cargo_bin("mansk").unwrap();
        command
            .env("HOME", &home)
            .env("XDG_CACHE_HOME", &cache_home)
            .args(["--manifest", manifest.to_str().unwrap()]);
        command
    };
    command().args(["update", "--yes"]).assert().success();
    let lock_before = fs::read(temp.path().join("skills.lock")).unwrap();
    let link = home.join(".claude/skills/one");
    let link_before = fs::read_link(&link).unwrap();
    let installed_document = link.join("SKILL.md");
    let installed_before = fs::read(&installed_document).unwrap();

    fs::write(temp.path().join("one/SKILL.md"), "changed").unwrap();
    command()
        .args(["sync", "--dry-run"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Noop"));
    assert_eq!(fs::read_link(&link).unwrap(), link_before);
    assert_eq!(fs::read(&installed_document).unwrap(), installed_before);

    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\"]\n[[skills]]\npath = \"one\"\n[[skills]]\npath = \"two\"\n",
    )
    .unwrap();
    command()
        .args(["update", "--dry-run", "--yes"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Link"));
    assert_eq!(
        fs::read(temp.path().join("skills.lock")).unwrap(),
        lock_before
    );
    assert_eq!(fs::read_link(&link).unwrap(), link_before);
    assert_eq!(fs::read(&installed_document).unwrap(), installed_before);
    assert!(!home.join(".claude/skills/two").exists());
}
