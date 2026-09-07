use std::{collections::BTreeMap, fs, path::Path, process::Command};

#[derive(Debug)]
pub struct Request {
    pub source: String,
    reference_path: Option<String>,
    direct: bool,
}

#[derive(Debug)]
pub struct Discovery {
    pub selector: String,
    pub commit: String,
    pub skills: Vec<String>,
}

impl Request {
    pub fn parse(input: &str) -> Result<Self, String> {
        let input = input.trim();
        let clean = input
            .split(['?', '#'])
            .next()
            .unwrap_or(input)
            .trim_end_matches('/');
        let (path, raw) = if let Some(path) = clean.strip_prefix("https://github.com/") {
            (path, false)
        } else if let Some(path) = clean.strip_prefix("https://raw.githubusercontent.com/") {
            (path, true)
        } else if !clean.contains(':') && !clean.starts_with('/') {
            (clean, false)
        } else {
            return Err(
                "expected a GitHub repository, folder, or SKILL.md URL, or owner/repo".into(),
            );
        };
        let parts = path
            .split('/')
            .map(decode_component)
            .collect::<Result<Vec<_>, _>>()?;
        if parts.len() < 2
            || !valid_repository_component(&parts[0])
            || !valid_repository_component(&parts[1])
        {
            return Err("GitHub source must include a valid owner and repository".into());
        }
        let repository = parts[1].strip_suffix(".git").unwrap_or(&parts[1]);
        if repository.is_empty() {
            return Err("GitHub repository name is empty".into());
        }
        let source = format!("https://github.com/{}/{repository}.git", parts[0]);
        let (reference_path, direct) = if raw {
            if parts.len() < 4 {
                return Err("raw GitHub URL must point to SKILL.md".into());
            }
            (Some(parts[2..].join("/")), true)
        } else if parts.len() == 2 {
            (None, false)
        } else {
            match parts[2].as_str() {
                "tree" if parts.len() >= 4 => (Some(parts[3..].join("/")), false),
                "blob" if parts.len() >= 5 => (Some(parts[3..].join("/")), true),
                _ => return Err("expected a GitHub tree or blob URL".into()),
            }
        };
        if direct && parts.last().map(String::as_str) != Some("SKILL.md") {
            return Err("direct file URL must point to SKILL.md".into());
        }
        Ok(Self {
            source,
            reference_path,
            direct,
        })
    }

    pub fn discover(
        &self,
        cache_root: &Path,
        locked: Option<(&str, &str)>,
    ) -> Result<Discovery, String> {
        // Resolve URL refs against advertised refs so slash-containing branch names
        // are distinguished from paths using the longest matching prefix.
        let refs = if self.reference_path.is_some() || locked.is_none() {
            git(None, &["ls-remote", "--symref", &self.source])?
        } else {
            String::new()
        };
        let (requested_selector, requested_commit, path) = self.resolve_reference(&refs, locked)?;
        let (selector, commit) = match locked {
            Some((selector, commit)) => {
                if self.reference_path.is_some()
                    && !same_selector(&requested_selector, selector)
                    && requested_commit != commit
                {
                    return Err(format!(
                        "GitHub URL requests `{requested_selector}`, but this repository is locked to `{selector}` ({commit}); update it separately first"
                    ));
                }
                (selector.to_owned(), commit.to_owned())
            }
            None => (requested_selector, requested_commit),
        };
        fs::create_dir_all(cache_root)
            .map_err(|error| format!("cannot create discovery cache: {error}"))?;
        let temporary = tempfile::Builder::new()
            .prefix("get-")
            .tempdir_in(cache_root)
            .map_err(|error| format!("cannot create discovery checkout: {error}"))?;
        let checkout = temporary.path();
        git(Some(checkout), &["init", "--quiet"])?;
        git(Some(checkout), &["remote", "add", "origin", &self.source])?;
        git(
            Some(checkout),
            &["fetch", "--quiet", "--depth=1", "origin", &commit],
        )?;
        git(
            Some(checkout),
            &["checkout", "--quiet", "--detach", "FETCH_HEAD"],
        )?;
        let actual = git(Some(checkout), &["rev-parse", "HEAD"])?;
        if actual != commit {
            return Err(format!(
                "discovery checkout resolved to {actual}, expected {commit}"
            ));
        }
        let mut directory = checkout.to_owned();
        for component in path.split('/').filter(|part| !part.is_empty()) {
            directory.push(component);
            let metadata = fs::symlink_metadata(&directory)
                .map_err(|error| format!("cannot inspect GitHub path `{path}`: {error}"))?;
            if metadata.file_type().is_symlink() {
                return Err(format!("GitHub path `{path}` contains a symlink"));
            }
        }
        let mut skills = Vec::new();
        if self.direct {
            if !fs::symlink_metadata(&directory).is_ok_and(|metadata| metadata.is_file()) {
                return Err(format!(
                    "GitHub path `{path}` is not a regular SKILL.md file"
                ));
            }
            skills.push(relative_directory(
                checkout,
                directory
                    .parent()
                    .ok_or("Skill document has no parent directory")?,
            )?);
        } else {
            if !directory.is_dir() {
                return Err(format!("GitHub path `{path}` is not a directory"));
            }
            discover_directory(checkout, &directory, &mut skills)?;
        }
        skills.sort();
        if skills.is_empty() {
            return Err("no SKILL.md files found beneath the requested GitHub location".into());
        }
        Ok(Discovery {
            selector,
            commit,
            skills,
        })
    }

    fn resolve_reference(
        &self,
        output: &str,
        locked: Option<(&str, &str)>,
    ) -> Result<(String, String, String), String> {
        let mut references = BTreeMap::new();
        let mut default = None;
        for line in output.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() == 3 && fields[0] == "ref:" && fields[2] == "HEAD" {
                default = Some(fields[1].to_owned());
            } else if fields.len() == 2 && is_commit(fields[0]) {
                references.insert(fields[1].to_owned(), fields[0].to_owned());
            }
        }
        let Some(location) = &self.reference_path else {
            if let Some((selector, commit)) = locked {
                return Ok((selector.into(), commit.into(), String::new()));
            }
            let selector =
                default.ok_or_else(|| "GitHub repository has no default branch".to_owned())?;
            let commit = references
                .get(&selector)
                .ok_or_else(|| "GitHub default branch has no commit".to_owned())?;
            return Ok((
                selector.trim_start_matches("refs/heads/").into(),
                commit.clone(),
                String::new(),
            ));
        };
        let first = location.split('/').next().unwrap_or_default();
        if is_commit(first) {
            return Ok((
                first.to_ascii_lowercase(),
                first.to_ascii_lowercase(),
                location.get(first.len() + 1..).unwrap_or("").into(),
            ));
        }
        let mut candidates = Vec::new();
        for (reference, commit) in &references {
            let Some(name) = reference
                .strip_prefix("refs/heads/")
                .or_else(|| reference.strip_prefix("refs/tags/"))
            else {
                continue;
            };
            if name.ends_with("^{}") {
                continue;
            }
            if location == name
                || location
                    .strip_prefix(name)
                    .is_some_and(|tail| tail.starts_with('/'))
            {
                let peeled = references
                    .get(&format!("{reference}^{{}}"))
                    .unwrap_or(commit);
                candidates.push((name.to_owned(), peeled.clone()));
            }
        }
        // A removed branch can still be used at the existing locked commit.
        if let Some((selector, commit)) = locked {
            let name = selector
                .strip_prefix("refs/heads/")
                .or_else(|| selector.strip_prefix("refs/tags/"))
                .unwrap_or(selector);
            if (location == name
                || location
                    .strip_prefix(name)
                    .is_some_and(|tail| tail.starts_with('/')))
                && !candidates.iter().any(|(candidate, _)| candidate == name)
            {
                candidates.push((name.to_owned(), commit.to_owned()));
            }
        }
        candidates.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
        let Some((selector, commit)) = candidates.first() else {
            return Err(format!(
                "no GitHub branch, tag, or full commit matches `{location}`"
            ));
        };
        if candidates
            .iter()
            .skip(1)
            .any(|(name, other)| name == selector && other != commit)
        {
            return Err(format!(
                "GitHub ref `{selector}` is ambiguous between a branch and tag"
            ));
        }
        Ok((
            selector.clone(),
            commit.clone(),
            location.get(selector.len() + 1..).unwrap_or("").into(),
        ))
    }
}

pub fn canonical_source(input: &str) -> Option<String> {
    let normalized = input
        .strip_prefix("git@github.com:")
        .or_else(|| input.strip_prefix("ssh://git@github.com/"))
        .map(|path| format!("https://github.com/{path}"));
    Request::parse(normalized.as_deref().unwrap_or(input))
        .ok()
        .map(|request| request.source.to_ascii_lowercase())
}

fn same_selector(left: &str, right: &str) -> bool {
    let short = |value: &str| {
        value
            .strip_prefix("refs/heads/")
            .or_else(|| value.strip_prefix("refs/tags/"))
            .unwrap_or(value)
            .to_owned()
    };
    short(left) == short(right)
}

fn valid_repository_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

fn decode_component(value: &str) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut index = 0;
    while index < value.len() {
        if value.as_bytes()[index] == b'%' {
            let encoded = value
                .get(index + 1..index + 3)
                .ok_or("invalid URL percent encoding")?;
            bytes
                .push(u8::from_str_radix(encoded, 16).map_err(|_| "invalid URL percent encoding")?);
            index += 3;
        } else {
            bytes.push(value.as_bytes()[index]);
            index += 1;
        }
    }
    let decoded = String::from_utf8(bytes).map_err(|_| "GitHub URL is not UTF-8")?;
    if decoded
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == ".." || part == ".git")
        || decoded.contains(['\\', '\0'])
    {
        return Err("GitHub URL contains an invalid path component".into());
    }
    Ok(decoded)
}

fn is_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn relative_directory(root: &Path, directory: &Path) -> Result<String, String> {
    let relative = directory
        .strip_prefix(root)
        .map_err(|error| error.to_string())?;
    if relative.as_os_str().is_empty() {
        return Ok(".".into());
    }
    relative
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| "skill path is not UTF-8".into())
}

fn discover_directory(
    root: &Path,
    directory: &Path,
    skills: &mut Vec<String>,
) -> Result<(), String> {
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
    {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry.file_name() == ".git" {
            continue;
        }
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        if kind.is_dir() {
            discover_directory(root, &entry.path(), skills)?;
        } else if kind.is_file() && entry.file_name() == "SKILL.md" {
            skills.push(relative_directory(root, directory)?);
        }
    }
    Ok(())
}

fn git(directory: Option<&Path>, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new("git");
    command.args(args).env("GIT_TERMINAL_PROMPT", "0");
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    let output = command
        .output()
        .map_err(|error| format!("failed to run git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args[0],
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_locations_and_rejects_escape_paths() {
        for input in [
            "owner/repo",
            "https://github.com/owner/repo.git",
            "https://github.com/owner/repo/tree/main/skills",
            "https://github.com/owner/repo/blob/main/skills/example/SKILL.md",
            "https://raw.githubusercontent.com/owner/repo/main/SKILL.md",
        ] {
            assert_eq!(
                Request::parse(input).unwrap().source,
                "https://github.com/owner/repo.git"
            );
        }
        for input in [
            "https://other.com/owner/repo",
            "https://github.com/owner/repo/tree/main/%2e%2e/secret",
            "https://github.com/owner/repo/blob/main/README.md",
            "owner",
            "https://github.com/owner/repo/tree/main/.git",
        ] {
            assert!(Request::parse(input).is_err(), "{input}");
        }
        assert_eq!(
            canonical_source("git@github.com:Owner/Repo.git"),
            Some("https://github.com/owner/repo.git".into())
        );
    }

    #[test]
    fn longest_ref_and_peeled_tag_resolution() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let refs = format!(
            "ref: refs/heads/main HEAD\n{a}\trefs/heads/main\n{b}\trefs/heads/main/feature\n{a}\trefs/tags/v1\n{b}\trefs/tags/v1^{{}}\n"
        );
        let request = Request::parse("https://github.com/o/r/tree/main/feature/skills").unwrap();
        assert_eq!(
            request.resolve_reference(&refs, None).unwrap(),
            ("main/feature".into(), b.clone(), "skills".into())
        );
        let request = Request::parse("https://github.com/o/r/blob/v1/SKILL.md").unwrap();
        assert_eq!(
            request.resolve_reference(&refs, None).unwrap(),
            ("v1".into(), b, "SKILL.md".into())
        );
    }

    #[test]
    fn discovers_local_repository_and_reuses_locked_commit() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(repo.join("skills/nested/example")).unwrap();
        fs::write(repo.join("SKILL.md"), "root").unwrap();
        fs::write(repo.join("skills/nested/example/SKILL.md"), "example").unwrap();
        git(Some(&repo), &["init", "-b", "main"]).unwrap();
        git(Some(&repo), &["add", "."]).unwrap();
        git(
            Some(&repo),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                "first",
            ],
        )
        .unwrap();
        let first = git(Some(&repo), &["rev-parse", "HEAD"]).unwrap();
        let mut request = Request::parse("o/r").unwrap();
        request.source = repo.to_str().unwrap().into();
        let cache = temp.path().join("cache");
        let found = request.discover(&cache, None).unwrap();
        assert_eq!(found.skills, [".", "skills/nested/example"]);
        assert_eq!(found.selector, "main");
        fs::create_dir(repo.join("new")).unwrap();
        fs::write(repo.join("new/SKILL.md"), "new").unwrap();
        git(Some(&repo), &["add", "."]).unwrap();
        git(
            Some(&repo),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                "second",
            ],
        )
        .unwrap();
        git(Some(&repo), &["branch", "other"]).unwrap();
        request.reference_path = Some("other/skills".into());
        assert!(
            request
                .discover(&cache, Some(("main", &first)))
                .unwrap_err()
                .contains("locked")
        );
        request.reference_path = Some("main/skills".into());
        let found = request.discover(&cache, Some(("main", &first))).unwrap();
        assert_eq!(found.commit, first);
        assert_eq!(found.skills, ["skills/nested/example"]);
        request.reference_path = Some("main/skills/nested/example/SKILL.md".into());
        request.direct = true;
        assert_eq!(
            request.discover(&cache, None).unwrap().skills,
            ["skills/nested/example"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn recursive_discovery_ignores_symlinks_and_git_metadata() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("actual")).unwrap();
        fs::create_dir_all(temp.path().join(".git")).unwrap();
        fs::write(temp.path().join("actual/SKILL.md"), "skill").unwrap();
        fs::write(temp.path().join(".git/SKILL.md"), "ignored").unwrap();
        symlink("actual", temp.path().join("alias")).unwrap();
        symlink("actual/SKILL.md", temp.path().join("SKILL.md")).unwrap();
        let mut skills = Vec::new();
        discover_directory(temp.path(), temp.path(), &mut skills).unwrap();
        assert_eq!(skills, ["actual"]);
    }
}
