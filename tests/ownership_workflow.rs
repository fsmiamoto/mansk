use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use std::fs;

#[cfg(unix)]
#[test]
fn sync_replaces_an_owned_link_to_a_different_cached_path() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache_home = temp.path().join("cache");
    let skill = temp.path().join("review");
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "review").unwrap();
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n[[skills]]\npath = \"review\"\n",
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
    let installed = home.join(".claude/skills/review");
    fs::remove_file(&installed).unwrap();
    let old_cache_path = cache_home.join("mansk/old/review");
    fs::create_dir_all(&old_cache_path).unwrap();
    symlink(&old_cache_path, &installed).unwrap();

    command()
        .arg("sync")
        .assert()
        .success()
        .stdout(predicates::str::contains("Remove").and(predicates::str::contains("Link")));

    assert_eq!(
        fs::read_link(installed).unwrap(),
        cache_home.join("mansk/local/review")
    );
}

#[cfg(unix)]
#[test]
fn missing_target_directories_are_created_only_when_applying_links() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache_home = temp.path().join("cache");
    let skill = temp.path().join("review");
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "review").unwrap();
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n[[skills]]\npath = \"review\"\n",
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

    command()
        .args(["update", "--dry-run", "--yes"])
        .assert()
        .success();
    assert!(!home.join(".claude").exists());
    assert!(!home.join(".agents").exists());

    command().args(["update", "--yes"]).assert().success();
    assert!(home.join(".claude/skills/review").is_symlink());
    assert!(!home.join(".agents").exists());
}

#[cfg(unix)]
#[test]
fn duplicate_names_on_overlapping_targets_fail_before_target_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache_home = temp.path().join("cache");
    let skill = temp.path().join("review");
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "review").unwrap();
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n[[skills]]\npath = \"review\"\n",
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
    let installed = home.join(".claude/skills/review");
    let destination_before = fs::read_link(&installed).unwrap();
    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n[[skills]]\npath = \"review\"\ntargets = [\"claude\"]\n[[skills]]\npath = \"review\"\ntargets = [\"claude\", \"agents\"]\n",
    )
    .unwrap();

    command()
        .arg("sync")
        .assert()
        .failure()
        .stderr(predicates::str::contains("duplicate skill name `review`"));

    assert_eq!(fs::read_link(installed).unwrap(), destination_before);
    assert!(!home.join(".agents").exists());
}

#[cfg(unix)]
#[test]
fn unmanaged_collision_fails_sync_without_applying_any_planned_removals() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache_home = temp.path().join("cache");
    for name in ["wanted", "stale"] {
        fs::create_dir_all(temp.path().join(name)).unwrap();
        fs::write(temp.path().join(name).join("SKILL.md"), name).unwrap();
    }
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n[[skills]]\npath = \"wanted\"\n[[skills]]\npath = \"stale\"\n",
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
    let target = home.join(".claude/skills");
    fs::remove_file(target.join("wanted")).unwrap();
    fs::create_dir(target.join("wanted")).unwrap();
    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n[[skills]]\npath = \"wanted\"\n",
    )
    .unwrap();

    command()
        .arg("sync")
        .assert()
        .failure()
        .stderr(predicates::str::contains("remove it manually"));

    assert!(target.join("wanted").is_dir());
    assert!(target.join("stale").is_symlink());
}

#[cfg(unix)]
#[test]
fn pruning_preserves_real_directories_and_links_outside_the_cache() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache_home = temp.path().join("cache");
    let skill = temp.path().join("owned");
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "owned").unwrap();
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n[[skills]]\npath = \"owned\"\n",
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
    let target = home.join(".claude/skills");
    let real = target.join("real");
    fs::create_dir(&real).unwrap();
    let external_source = temp.path().join("external");
    fs::create_dir(&external_source).unwrap();
    let external_link = target.join("external");
    symlink(&external_source, &external_link).unwrap();

    fs::write(&manifest, "schema = 1\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n").unwrap();
    command().arg("sync").assert().success();

    assert!(real.is_dir());
    assert_eq!(fs::read_link(external_link).unwrap(), external_source);
    assert!(!target.join("owned").exists());
}

#[cfg(unix)]
#[test]
fn sync_prunes_the_final_removed_skill() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache_home = temp.path().join("cache");
    let skill = temp.path().join("review");
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "review").unwrap();
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n[[skills]]\npath = \"review\"\n",
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

    fs::write(&manifest, "schema = 1\ndefault-targets = [\"claude\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n").unwrap();
    command()
        .arg("sync")
        .assert()
        .success()
        .stdout(predicates::str::contains("Remove"));

    assert!(!home.join(".claude/skills/review").exists());
}

#[cfg(unix)]
#[test]
fn sync_prunes_only_the_removed_skills_owned_links_with_a_stale_lock() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let cache_home = temp.path().join("cache");
    for name in ["keep", "remove"] {
        let skill = temp.path().join(name);
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), name).unwrap();
    }
    let manifest = temp.path().join("skills.toml");
    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\", \"agents\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n[[skills]]\npath = \"keep\"\n[[skills]]\npath = \"remove\"\n",
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

    fs::write(
        &manifest,
        "schema = 1\ndefault-targets = [\"claude\", \"agents\"]\n[targets]\nclaude = \".claude/skills\"\nagents = \".agents/skills\"\n[[skills]]\npath = \"keep\"\n",
    )
    .unwrap();
    command()
        .arg("sync")
        .assert()
        .success()
        .stdout(predicates::str::contains("Remove"));

    for target in [".claude/skills", ".agents/skills"] {
        assert!(home.join(target).join("keep").is_symlink());
        assert!(!home.join(target).join("remove").exists());
    }
    let lock = fs::read_to_string(temp.path().join("skills.lock")).unwrap();
    assert!(
        lock.contains("remove"),
        "sync must tolerate stale attribution"
    );
}
