use assert_cmd::Command;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command as GitCommand,
};

const SOURCE: &str = "https://github.com/test/skills.git";
const MANIFEST: &str = "# Keep this comment\nschema = 1\ndefault-targets = [\"claude\"] # Default agent\n\n[targets]\nclaude = \".claude/skills\"\n";

fn git(repo: &Path, args: &[&str]) -> String {
    let output = GitCommand::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

struct Fixture {
    temp: tempfile::TempDir,
    repo: PathBuf,
    manifest: PathBuf,
}

impl Fixture {
    fn new(paths: &[&str]) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("upstream");
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        git(&repo, &["config", "user.name", "Test"]);
        for path in paths {
            fs::create_dir_all(repo.join(path)).unwrap();
            fs::write(
                repo.join(path).join("SKILL.md"),
                format!("Skill at {path}\n"),
            )
            .unwrap();
        }
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "initial"]);
        let manifest = temp.path().join("skills.toml");
        fs::write(&manifest, MANIFEST).unwrap();
        Self {
            temp,
            repo,
            manifest,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("mansk").unwrap();
        command
            .current_dir(self.temp.path())
            .env("HOME", self.temp.path().join("home"))
            .env("XDG_CACHE_HOME", self.temp.path().join("cache"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CONFIG_COUNT", "1")
            .env(
                "GIT_CONFIG_KEY_0",
                format!("url.file://{}.insteadOf", self.repo.display()),
            )
            .env("GIT_CONFIG_VALUE_0", SOURCE)
            .args(["--manifest", self.manifest.to_str().unwrap()]);
        command
    }

    fn installed(&self, name: &str) -> PathBuf {
        self.temp.path().join("home/.claude/skills").join(name)
    }

    fn manifest_text(&self) -> String {
        fs::read_to_string(&self.manifest).unwrap()
    }

    fn manifest_value(&self) -> toml::Value {
        toml::from_str(&self.manifest_text()).unwrap()
    }

    fn lock(&self) -> Value {
        serde_json::from_slice(&fs::read(self.temp.path().join("skills.lock")).unwrap()).unwrap()
    }

    fn commit(&self) -> String {
        git(&self.repo, &["add", "."]);
        git(&self.repo, &["commit", "-m", "advance"]);
        git(&self.repo, &["rev-parse", "HEAD"])
    }

    fn assert_untouched(&self) {
        assert_eq!(self.manifest_text(), MANIFEST);
        assert!(!self.temp.path().join("skills.lock").exists());
        assert!(!self.installed("").exists());
    }
}

#[test]
fn repo_get_discovers_recursively_preserves_comments_and_installs_supporting_files() {
    let fixture = Fixture::new(&["skills/review", "deep/nested/explain"]);
    fs::create_dir_all(fixture.repo.join("skills/review/scripts")).unwrap();
    fs::write(
        fixture.repo.join("skills/review/scripts/run.sh"),
        "echo review\n",
    )
    .unwrap();
    let commit = fixture.commit();
    fixture
        .command()
        .args(["get", "https://github.com/test/skills", "--all"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(fixture.installed("review/scripts/run.sh")).unwrap(),
        "echo review\n"
    );
    assert!(fixture.installed("explain/SKILL.md").exists());
    assert_eq!(fixture.lock()["git"][SOURCE], commit);
    assert!(fixture.manifest_text().contains("# Keep this comment"));
    assert!(fixture.manifest_text().contains("# Default agent"));
    let manifest = fixture.manifest_value();
    assert_eq!(manifest["skills"].as_array().unwrap().len(), 2);
    assert!(manifest.get("collections").is_none());
    fixture.command().arg("sync").assert().success();
}

#[test]
fn tree_url_limits_recursive_discovery_to_selected_directory() {
    let fixture = Fixture::new(&["skills/review", "skills/nested/explain", "other/outside"]);
    fixture
        .command()
        .args([
            "get",
            "https://github.com/test/skills/tree/main/skills",
            "--all",
        ])
        .assert()
        .success();
    assert!(fixture.installed("review/SKILL.md").exists());
    assert!(fixture.installed("explain/SKILL.md").exists());
    assert!(!fixture.installed("outside").exists());
}

#[test]
fn blob_url_installs_only_the_containing_skill_without_a_picker() {
    let fixture = Fixture::new(&["skills/review", "skills/explain"]);
    fixture
        .command()
        .args([
            "get",
            "https://github.com/test/skills/blob/main/skills/review/SKILL.md",
        ])
        .assert()
        .success();
    assert!(fixture.installed("review/SKILL.md").exists());
    assert!(!fixture.installed("explain").exists());
}

#[test]
fn shorthand_single_skill_and_repeated_get_are_idempotent() {
    let fixture = Fixture::new(&["skills/review"]);
    fixture
        .command()
        .args(["get", "test/skills"])
        .assert()
        .success();
    let manifest = fixture.manifest_text();
    let lock = fixture.lock();
    fixture
        .command()
        .args(["get", "test/skills"])
        .assert()
        .success();
    assert_eq!(fixture.manifest_text(), manifest);
    assert_eq!(fixture.lock(), lock);
    assert!(fixture.installed("review/SKILL.md").exists());
}

#[test]
fn repeated_skill_flags_save_only_exact_selections() {
    let fixture = Fixture::new(&["skills/review", "skills/explain", "skills/unused"]);
    fixture
        .command()
        .args([
            "get",
            "test/skills",
            "--skill",
            "skills/review",
            "--skill",
            "skills/explain",
        ])
        .assert()
        .success();
    assert_eq!(
        fixture.manifest_value()["skills"].as_array().unwrap().len(),
        2
    );
    assert!(fixture.installed("review/SKILL.md").exists());
    assert!(fixture.installed("explain/SKILL.md").exists());
    assert!(!fixture.installed("unused").exists());
}

#[test]
fn multiple_skills_without_a_terminal_or_selection_leave_state_unchanged() {
    let fixture = Fixture::new(&["skills/review", "skills/explain"]);
    fixture
        .command()
        .args(["get", "test/skills"])
        .assert()
        .failure();
    fixture.assert_untouched();
}

#[test]
fn missing_explicit_selection_leaves_state_unchanged() {
    let fixture = Fixture::new(&["skills/review"]);
    fixture
        .command()
        .args(["get", "test/skills", "--skill", "skills/missing"])
        .assert()
        .failure();
    fixture.assert_untouched();
}

#[test]
fn conflicting_install_names_leave_state_unchanged() {
    let fixture = Fixture::new(&["first/review", "second/review"]);
    fixture
        .command()
        .args(["get", "test/skills", "--all"])
        .assert()
        .failure();
    fixture.assert_untouched();
}

#[test]
fn adding_a_skill_reuses_the_existing_repo_lock_after_branch_advances() {
    let fixture = Fixture::new(&["skills/review", "skills/explain"]);
    let original_commit = git(&fixture.repo, &["rev-parse", "HEAD"]);
    fixture
        .command()
        .args(["get", "test/skills", "--skill", "skills/review"])
        .assert()
        .success();
    fs::write(fixture.repo.join("skills/explain/SKILL.md"), "New version").unwrap();
    fixture.commit();
    fixture
        .command()
        .args(["get", "test/skills", "--skill", "skills/explain"])
        .assert()
        .success();
    assert_eq!(fixture.lock()["git"][SOURCE], original_commit);
    assert_eq!(
        fs::read_to_string(fixture.installed("explain/SKILL.md")).unwrap(),
        "Skill at skills/explain\n"
    );
    fixture.command().arg("sync").assert().success();
}

#[test]
fn explicit_conflicting_commit_leaves_existing_installation_unchanged() {
    let fixture = Fixture::new(&["skills/review", "skills/explain"]);
    fixture
        .command()
        .args(["get", "test/skills", "--skill", "skills/review"])
        .assert()
        .success();
    let manifest = fixture.manifest_text();
    let lock = fixture.lock();
    fs::write(fixture.repo.join("skills/explain/SKILL.md"), "New version").unwrap();
    let new_commit = fixture.commit();
    let url = format!("https://github.com/test/skills/tree/{new_commit}/skills/explain");
    fixture.command().args(["get", &url]).assert().failure();
    assert_eq!(fixture.manifest_text(), manifest);
    assert_eq!(fixture.lock(), lock);
    assert!(!fixture.installed("explain").exists());
}

#[test]
fn get_does_not_advance_an_unrelated_existing_repo() {
    let fixture = Fixture::new(&["skills/review"]);
    let other = fixture.temp.path().join("other");
    fs::create_dir_all(other.join("existing")).unwrap();
    git(&other, &["init", "-b", "main"]);
    git(&other, &["config", "user.email", "test@example.com"]);
    git(&other, &["config", "user.name", "Test"]);
    fs::write(other.join("existing/SKILL.md"), "Original").unwrap();
    git(&other, &["add", "."]);
    git(&other, &["commit", "-m", "initial"]);
    let original_commit = git(&other, &["rev-parse", "HEAD"]);
    let source = format!("file://{}", other.display());
    fs::write(&fixture.manifest, format!("{MANIFEST}\n[[skills]]\nsource = {source:?}\nselector = \"main\"\npath = \"existing\"\n")).unwrap();
    fixture
        .command()
        .args(["update", "--yes"])
        .assert()
        .success();
    fs::write(other.join("existing/SKILL.md"), "Advanced").unwrap();
    git(&other, &["add", "."]);
    git(&other, &["commit", "-m", "advance"]);
    fixture
        .command()
        .args(["get", "test/skills"])
        .assert()
        .success();
    assert_eq!(fixture.lock()["git"][&source], original_commit);
    assert_eq!(
        fs::read_to_string(fixture.installed("existing/SKILL.md")).unwrap(),
        "Original"
    );
    assert!(fixture.installed("review/SKILL.md").exists());
}

#[test]
fn root_skill_uses_repository_name_and_dot_path() {
    let fixture = Fixture::new(&["."]);
    fixture
        .command()
        .args(["get", "test/skills"])
        .assert()
        .success();
    assert!(fixture.installed("skills/SKILL.md").exists());
    assert_eq!(
        fixture.manifest_value()["skills"][0]["path"].as_str(),
        Some(".")
    );
    fixture.command().arg("sync").assert().success();
}

#[test]
fn all_does_not_opt_into_future_skills_on_update() {
    let fixture = Fixture::new(&["skills/review", "skills/explain"]);
    fixture
        .command()
        .args(["get", "test/skills", "--all"])
        .assert()
        .success();
    fs::create_dir_all(fixture.repo.join("skills/future")).unwrap();
    fs::write(fixture.repo.join("skills/future/SKILL.md"), "Future skill").unwrap();
    fixture.commit();
    fixture
        .command()
        .args(["update", "--yes"])
        .assert()
        .success();
    assert!(!fixture.installed("future").exists());
    assert_eq!(
        fixture.manifest_value()["skills"].as_array().unwrap().len(),
        2
    );
}

#[cfg(unix)]
#[test]
fn manifest_symlink_is_preserved_and_underlying_file_is_updated() {
    let fixture = Fixture::new(&["skills/review"]);
    let underlying = fixture.temp.path().join("dotfiles/skills.toml");
    fs::create_dir_all(underlying.parent().unwrap()).unwrap();
    fs::rename(&fixture.manifest, &underlying).unwrap();
    std::os::unix::fs::symlink(&underlying, &fixture.manifest).unwrap();
    fixture
        .command()
        .args(["get", "test/skills"])
        .assert()
        .success();
    assert!(fixture.manifest.is_symlink());
    assert_eq!(fs::read_link(&fixture.manifest).unwrap(), underlying);
    let saved: toml::Value = toml::from_str(&fs::read_to_string(underlying).unwrap()).unwrap();
    assert_eq!(saved["skills"][0]["path"].as_str(), Some("skills/review"));
    assert!(fixture.installed("review/SKILL.md").exists());
    fixture.command().arg("sync").assert().success();
}

#[test]
fn empty_inline_skills_array_accepts_new_entries() {
    let fixture = Fixture::new(&["skills/review"]);
    fs::write(&fixture.manifest, format!("skills = []\n{MANIFEST}")).unwrap();
    fixture
        .command()
        .args(["get", "test/skills"])
        .assert()
        .success();
    let saved = fixture.manifest_value();
    assert_eq!(saved["skills"].as_array().unwrap().len(), 1);
    assert_eq!(saved["skills"][0]["path"].as_str(), Some("skills/review"));
    assert!(fixture.installed("review/SKILL.md").exists());
    fixture.command().arg("sync").assert().success();
}

#[test]
fn existing_collection_member_is_skipped_and_new_selection_preserves_collection_lock() {
    let fixture = Fixture::new(&["skills/review", "outside/explain"]);
    fs::write(&fixture.manifest, format!("{MANIFEST}\n[[collections]]\nsource = {SOURCE:?}\nselector = \"main\"\nroot = \"skills\"\n")).unwrap();
    fixture
        .command()
        .args(["update", "--yes"])
        .assert()
        .success();
    let original_manifest = fixture.manifest_text();
    let original_lock = fixture.lock();
    fixture
        .command()
        .args(["get", "test/skills", "--skill", "skills/review"])
        .assert()
        .success();
    assert_eq!(fixture.manifest_text(), original_manifest);
    assert_eq!(fixture.lock(), original_lock);
    fs::create_dir_all(fixture.repo.join("skills/future")).unwrap();
    fs::write(
        fixture.repo.join("skills/future/SKILL.md"),
        "New collection member",
    )
    .unwrap();
    fs::write(fixture.repo.join("outside/explain/SKILL.md"), "New version").unwrap();
    fixture.commit();
    fixture
        .command()
        .args([
            "get",
            "test/skills",
            "--skill",
            "skills/review",
            "--skill",
            "outside/explain",
        ])
        .assert()
        .success();
    let updated_lock = fixture.lock();
    assert_eq!(updated_lock["collections"], original_lock["collections"]);
    assert_eq!(
        updated_lock["git"][SOURCE],
        original_lock["collections"][0]["commit"]
    );
    let saved = fixture.manifest_value();
    assert_eq!(saved["skills"].as_array().unwrap().len(), 1);
    assert_eq!(saved["skills"][0]["path"].as_str(), Some("outside/explain"));
    assert!(fixture.installed("review/SKILL.md").exists());
    assert_eq!(
        fs::read_to_string(fixture.installed("explain/SKILL.md")).unwrap(),
        "Skill at outside/explain\n"
    );
    assert!(!fixture.installed("future").exists());
    fixture.command().arg("sync").assert().success();
}

#[test]
fn raw_github_skill_url_installs_only_its_containing_directory() {
    let fixture = Fixture::new(&["skills/review", "skills/explain"]);
    fixture
        .command()
        .args([
            "get",
            "https://raw.githubusercontent.com/test/skills/main/skills/review/SKILL.md",
        ])
        .assert()
        .success();
    assert!(fixture.installed("review/SKILL.md").exists());
    assert!(!fixture.installed("explain").exists());
    assert_eq!(
        fixture.manifest_value()["skills"][0]["path"].as_str(),
        Some("skills/review")
    );
}

#[test]
fn first_run_without_a_terminal_explains_required_setup_without_creating_files() {
    let fixture = Fixture::new(&["skills/review"]);
    fs::remove_file(&fixture.manifest).unwrap();
    fixture
        .command()
        .args(["get", "test/skills", "--all"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("No manifest found"))
        .stderr(predicates::str::contains("default-targets"));
    assert!(!fixture.manifest.exists());
    assert!(!fixture.temp.path().join("skills.lock").exists());
    assert!(!fixture.temp.path().join("home").exists());
    assert!(!fixture.temp.path().join("cache").exists());
}

#[test]
fn root_collection_member_is_skipped_when_selected_with_dot_prefix() {
    let fixture = Fixture::new(&["review", "nested/explain"]);
    fs::write(
        &fixture.manifest,
        format!("{MANIFEST}\n[[collections]]\nsource = {SOURCE:?}\nselector = \"main\"\n"),
    )
    .unwrap();
    fixture
        .command()
        .args(["update", "--yes"])
        .assert()
        .success();
    fixture
        .command()
        .args([
            "get",
            "test/skills",
            "--skill",
            "./review",
            "--skill",
            "nested/explain",
        ])
        .assert()
        .success();
    assert_eq!(
        fixture.manifest_value()["skills"].as_array().unwrap().len(),
        1
    );
    assert!(fixture.installed("review/SKILL.md").exists());
    assert!(fixture.installed("explain/SKILL.md").exists());
    fixture.command().arg("sync").assert().success();
}
