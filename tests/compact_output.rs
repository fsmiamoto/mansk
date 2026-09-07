use assert_cmd::Command;
use std::{fs, path::Path};

fn command(root: &Path) -> Command {
    let mut cmd = Command::cargo_bin("mansk").unwrap();
    cmd.env("HOME", root.join("home"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .args(["--manifest", root.join("skills.toml").to_str().unwrap()]);
    cmd
}

#[test]
fn compact_output_tracks_local_refresh_and_keeps_preview_read_only() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir(root.join("review")).unwrap();
    fs::write(root.join("review/SKILL.md"), "one").unwrap();
    fs::write(root.join("skills.toml"), "schema = 1\ndefault-targets = [\"claude\", \"pi\"]\n[targets]\nclaude = \".claude/skills\"\npi = \".pi/skills\"\n[[skills]]\npath = \"review\"\n").unwrap();
    let output = command(root).args(["update", "--yes"]).output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        text.lines().filter(|line| line.contains("review")).count(),
        1,
        "{text}"
    );
    assert!(
        text.contains("Add") && text.contains("claude, pi"),
        "{text}"
    );
    assert!(!text.contains(root.to_str().unwrap()), "{text}");
    let output = command(root).arg("update").output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Everything is up to date · 1 skill across 2 agents\n"
    );

    fs::write(root.join("review/SKILL.md"), "two").unwrap();
    let output = command(root).args(["sync", "--dry-run"]).output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("Update") && text.contains("Preview only"),
        "{text}"
    );
    assert_eq!(
        fs::read_to_string(root.join("home/.claude/skills/review/SKILL.md")).unwrap(),
        "one"
    );
    let output = command(root).arg("sync").output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Update") && text.contains("2"), "{text}");
    assert_eq!(
        fs::read_to_string(root.join("home/.claude/skills/review/SKILL.md")).unwrap(),
        "two"
    );
}
