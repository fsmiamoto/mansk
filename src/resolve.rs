use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

use crate::{lock::LockedCollection, manifest::Manifest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSkill {
    pub name: String,
    pub path: PathBuf,
    pub targets: Vec<String>,
}

pub fn cache_home_from_env() -> Result<PathBuf, String> {
    if let Some(cache_home) = std::env::var_os("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(cache_home).join("mansk"));
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".cache/mansk"))
        .ok_or_else(|| "cannot locate the cache: neither XDG_CACHE_HOME nor HOME is set".into())
}

pub fn local_skills(
    manifest: &Manifest,
    manifest_path: &Path,
    cache_root: &Path,
    refresh_cache: bool,
) -> Result<Vec<ResolvedSkill>, String> {
    let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let mut sources = Vec::new();
    let mut names = HashSet::new();

    // Validate every local source before changing even the cache. More importantly,
    // resolution completes before the caller is allowed to mutate any target.
    for skill in manifest
        .skills
        .iter()
        .filter(|skill| skill.source.is_none())
    {
        let source = manifest_dir.join(&skill.path);
        if !source.join("SKILL.md").is_file() {
            return Err(format!(
                "local skill {} is missing SKILL.md",
                source.display()
            ));
        }
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                format!(
                    "local skill path {} has no valid directory name",
                    source.display()
                )
            })?
            .to_owned();
        if !names.insert(name.clone()) {
            return Err(format!("duplicate skill name `{name}`"));
        }
        sources.push((
            source,
            name,
            skill
                .targets
                .clone()
                .unwrap_or_else(|| manifest.default_targets.clone()),
        ));
    }

    let local_root = cache_root.join("local");
    let staging_root = cache_root.join(".staging/local");
    let mut resolved = Vec::with_capacity(sources.len());
    for (source, name, targets) in sources {
        let destination = local_root.join(&name);
        if refresh_cache {
            for directory in [&local_root, &staging_root] {
                fs::create_dir_all(directory).map_err(|error| {
                    format!("failed to create cache {}: {error}", directory.display())
                })?;
            }
            let temporary = staging_root.join(&name);
            remove_if_exists(&temporary)?;
            copy_directory(&source, &temporary)?;
            remove_if_exists(&destination)?;
            fs::rename(&temporary, &destination).map_err(|error| {
                format!(
                    "failed to refresh cached skill {}: {error}",
                    destination.display()
                )
            })?;
        }
        resolved.push(ResolvedSkill {
            name,
            path: destination,
            targets,
        });
    }
    Ok(resolved)
}

pub fn resolve_git_selectors(manifest: &Manifest) -> Result<BTreeMap<String, String>, String> {
    let mut requests = Vec::new();
    for (index, skill) in manifest.skills.iter().enumerate() {
        let Some(source) = skill.source.as_deref() else {
            continue;
        };
        let selector = skill.selector.as_deref().ok_or_else(|| {
            format!(
                "cannot resolve Git skill {} from `{source}` without a selector",
                index + 1
            )
        })?;
        requests.push((source, selector));
    }
    for collection in &manifest.collections {
        requests.push((collection.source.as_str(), collection.selector.as_str()));
    }

    let mut commits = BTreeMap::new();
    for (source, selector) in requests {
        let commit = resolve_selector(source, selector)?;
        if let Some(previous) = commits.insert(source.to_owned(), commit.clone()) {
            if previous != commit {
                return Err(format!(
                    "repository `{source}` resolves to different commits ({previous} and {commit})"
                ));
            }
        }
    }
    Ok(commits)
}

pub fn discover_collections(
    manifest: &Manifest,
    commits: &BTreeMap<String, String>,
    cache_root: &Path,
) -> Result<Vec<LockedCollection>, String> {
    if manifest.collections.is_empty() {
        return Ok(Vec::new());
    }
    let staging_root = cache_root.join(".staging/git");
    remove_if_exists(&staging_root)?;
    let result = discover_collections_in(manifest, commits, &staging_root);
    let cleanup = remove_if_exists(&staging_root);
    match (result, cleanup) {
        (Ok(collections), Ok(())) => Ok(collections),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn discover_collections_in(
    manifest: &Manifest,
    commits: &BTreeMap<String, String>,
    checkout_root: &Path,
) -> Result<Vec<LockedCollection>, String> {
    let mut checkouts = HashMap::new();
    for collection in &manifest.collections {
        let commit = commits.get(&collection.source).ok_or_else(|| {
            format!(
                "resolved commits do not cover collection source `{}`",
                collection.source
            )
        })?;
        if !is_full_object_id(commit) {
            return Err(format!(
                "resolved collection commit for `{}` is not a full commit",
                collection.source
            ));
        }
        if !checkouts.contains_key(collection.source.as_str()) {
            let checkout = checkout_root.join(repository_key(&collection.source));
            ensure_checkout(&collection.source, commit, &checkout)?;
            checkouts.insert(collection.source.as_str(), checkout);
        }
    }

    let mut locked = Vec::with_capacity(manifest.collections.len());
    for collection in &manifest.collections {
        let checkout = &checkouts[collection.source.as_str()];
        let root = collection_root(checkout, collection.root.as_deref(), &collection.source)?;
        let entries = fs::read_dir(&root).map_err(|error| {
            format!(
                "collection root {} for `{}` cannot be read: {error}",
                root.display(),
                collection.source
            )
        })?;
        let mut members = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!("failed to read collection root {}: {error}", root.display())
            })?;
            let file_type = entry.file_type().map_err(|error| {
                format!(
                    "failed to inspect collection entry {}: {error}",
                    entry.path().display()
                )
            })?;
            if !file_type.is_dir() {
                continue;
            }
            let document = entry.path().join("SKILL.md");
            if fs::symlink_metadata(&document).is_ok_and(|metadata| metadata.file_type().is_file())
            {
                let name = entry.file_name().into_string().map_err(|_| {
                    format!(
                        "collection member {} has a non-UTF-8 name",
                        entry.path().display()
                    )
                })?;
                members.push(name);
            }
        }
        members.sort();
        locked.push(LockedCollection {
            source: collection.source.clone(),
            root: collection.root.clone(),
            commit: commits[&collection.source].clone(),
            members,
        });
    }
    Ok(locked)
}

pub fn git_skills(
    manifest: &Manifest,
    commits: &BTreeMap<String, String>,
    collections: &[LockedCollection],
    cache_root: &Path,
    permanent: bool,
) -> Result<Vec<ResolvedSkill>, String> {
    if !manifest.skills.iter().any(|skill| skill.source.is_some()) && collections.is_empty() {
        return Ok(Vec::new());
    }

    let permanent_root = cache_root.join("git");
    if permanent {
        return git_skills_in(
            manifest,
            commits,
            collections,
            &permanent_root,
            &permanent_root,
        );
    }

    let staging_root = cache_root.join(".staging/git");
    remove_if_exists(&staging_root)?;
    let result = git_skills_in(
        manifest,
        commits,
        collections,
        &staging_root,
        &permanent_root,
    );
    let cleanup = remove_if_exists(&staging_root);
    match (result, cleanup) {
        (Ok(skills), Ok(())) => Ok(skills),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn git_skills_in(
    manifest: &Manifest,
    commits: &BTreeMap<String, String>,
    collections: &[LockedCollection],
    checkout_root: &Path,
    permanent_root: &Path,
) -> Result<Vec<ResolvedSkill>, String> {
    let git_skills: Vec<_> = manifest
        .skills
        .iter()
        .filter_map(|skill| skill.source.as_ref().map(|source| (skill, source)))
        .collect();
    let mut requested = BTreeMap::new();
    for (_, source) in &git_skills {
        let commit = commits
            .get(*source)
            .ok_or_else(|| format!("skills.lock does not cover Git source `{source}`"))?;
        requested.insert(*source, commit);
    }
    for collection in collections {
        if let Some(explicit_commit) = commits.get(&collection.source) {
            if explicit_commit != &collection.commit {
                return Err(format!(
                    "skills.lock records inconsistent commits for repository `{}`",
                    collection.source
                ));
            }
        }
        if let Some(previous) = requested.insert(&collection.source, &collection.commit) {
            if previous != &collection.commit {
                return Err(format!(
                    "skills.lock records inconsistent commits for repository `{}`",
                    collection.source
                ));
            }
        }
    }

    let mut checkouts = HashMap::new();
    for (source, commit) in requested {
        if !is_full_object_id(commit) {
            return Err(format!(
                "skills.lock must record a full commit for Git source `{source}`"
            ));
        }
        let key = repository_key(source);
        let checkout = checkout_root.join(&key);
        ensure_checkout(source, commit, &checkout)?;
        checkouts.insert(source.as_str(), (key, checkout));
    }

    let mut names = HashSet::new();
    let mut resolved = Vec::new();
    for (skill, source) in git_skills {
        let (key, checkout) = checkouts
            .get(source.as_str())
            .ok_or_else(|| format!("skills.lock does not cover Git source `{source}`"))?;
        let relative = validate_repository_path(&skill.path)?;
        let validation_path = checkout.join(relative);
        validate_git_skill_path(checkout, &validation_path, &skill.path)?;
        let name = relative
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                format!(
                    "Git skill path `{}` has no valid directory name",
                    skill.path
                )
            })?
            .to_owned();
        if !names.insert(name.clone()) {
            return Err(format!("duplicate skill name `{name}`"));
        }
        resolved.push(ResolvedSkill {
            name,
            path: permanent_root.join(key).join(relative),
            targets: skill
                .targets
                .clone()
                .unwrap_or_else(|| manifest.default_targets.clone()),
        });
    }

    for (collection, locked) in manifest.collections.iter().zip(collections) {
        if collection.source != locked.source || collection.root != locked.root {
            return Err(format!(
                "skills.lock does not cover collection from `{}` with root `{}`",
                collection.source,
                collection.root.as_deref().unwrap_or(".")
            ));
        }
        let (key, checkout) = checkouts.get(collection.source.as_str()).ok_or_else(|| {
            format!(
                "skills.lock does not cover collection source `{}`",
                collection.source
            )
        })?;
        let root = collection_root(checkout, collection.root.as_deref(), &collection.source)?;
        let relative_root = root.strip_prefix(checkout).map_err(|_| {
            format!(
                "collection root {} for `{}` is outside Git checkout {}",
                root.display(),
                collection.source,
                checkout.display()
            )
        })?;
        for member in &locked.members {
            let member_path = validate_collection_member(member)?;
            let validation_path = root.join(member_path);
            validate_git_skill_path(checkout, &validation_path, member).map_err(|error| {
                format!(
                    "skills.lock collection member `{member}` for `{}` is inconsistent: {error}",
                    collection.source
                )
            })?;
            if !names.insert(member.clone()) {
                return Err(format!("duplicate skill name `{member}`"));
            }
            resolved.push(ResolvedSkill {
                name: member.clone(),
                path: permanent_root
                    .join(key)
                    .join(relative_root)
                    .join(member_path),
                targets: manifest.default_targets.clone(),
            });
        }
    }
    Ok(resolved)
}

fn resolve_selector(source: &str, selector: &str) -> Result<String, String> {
    if is_full_object_id(selector) {
        return Ok(selector.to_ascii_lowercase());
    }
    let peeled = format!("{selector}^{{}}");
    let output = git_output(None, &["ls-remote", source, selector, &peeled])?;
    let mut ordinary = Vec::new();
    let mut peeled_commits = Vec::new();
    for line in output.lines() {
        let mut fields = line.split_whitespace();
        let Some(commit) = fields.next() else {
            continue;
        };
        let reference = fields.next().unwrap_or_default();
        if reference.ends_with("^{}") {
            peeled_commits.push(commit.to_owned());
        } else {
            ordinary.push(commit.to_owned());
        }
    }
    let candidates = if peeled_commits.is_empty() {
        ordinary
    } else {
        peeled_commits
    };
    let mut unique = candidates.into_iter().collect::<HashSet<_>>().into_iter();
    let Some(commit) = unique.next() else {
        return Err(format!(
            "Git selector `{selector}` was not found in repository `{source}`"
        ));
    };
    if unique.next().is_some() {
        return Err(format!(
            "Git selector `{selector}` is ambiguous in repository `{source}`"
        ));
    }
    Ok(commit)
}

fn is_full_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn repository_key(source: &str) -> String {
    // Stable FNV-1a keeps repository URLs out of path syntax while making the
    // one-working-copy-per-source rule visible in the cache layout.
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in source.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn ensure_checkout(source: &str, commit: &str, checkout: &Path) -> Result<(), String> {
    if checkout.join(".git").is_dir() {
        if git_output(
            Some(checkout),
            &["cat-file", "-e", &format!("{commit}^{{commit}}")],
        )
        .is_err()
        {
            git_output(Some(checkout), &["fetch", "--no-tags", "origin", commit])?;
        }
    } else {
        remove_if_exists(checkout)?;
        fs::create_dir_all(checkout).map_err(|error| {
            format!("failed to create Git cache {}: {error}", checkout.display())
        })?;
        git_output(Some(checkout), &["init"])?;
        git_output(Some(checkout), &["remote", "add", "origin", source])?;
        git_output(Some(checkout), &["fetch", "--no-tags", "origin", commit])?;
    }
    git_output(Some(checkout), &["checkout", "--detach", "--force", commit])?;
    git_output(Some(checkout), &["clean", "-fdx"])?;
    let actual = git_output(Some(checkout), &["rev-parse", "HEAD"])?;
    if actual != commit {
        return Err(format!(
            "Git checked out {actual} instead of requested commit {commit} for `{source}`"
        ));
    }
    Ok(())
}

fn git_output(current_dir: Option<&Path>, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new("git");
    command.args(args);
    if let Some(directory) = current_dir {
        command.current_dir(directory);
    }
    let output = command
        .output()
        .map_err(|error| format!("failed to run git {}: {error}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(format!(
            "git {} failed{}{}",
            args.join(" "),
            if stderr.is_empty() { "" } else { ": " },
            stderr
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn collection_root(
    checkout: &Path,
    configured_root: Option<&str>,
    source: &str,
) -> Result<PathBuf, String> {
    let root = match configured_root {
        Some(root) => checkout.join(validate_repository_path(root)?),
        None => checkout.to_owned(),
    };
    let repository = fs::canonicalize(checkout)
        .map_err(|error| format!("failed to inspect collection repository `{source}`: {error}"))?;
    let canonical = fs::canonicalize(&root).map_err(|error| {
        format!(
            "collection root {} for `{source}` does not exist: {error}",
            configured_root.unwrap_or(".")
        )
    })?;
    if !canonical.starts_with(&repository) {
        return Err(format!(
            "collection root {} for `{source}` escapes the repository",
            configured_root.unwrap_or(".")
        ));
    }
    if !canonical.is_dir() {
        return Err(format!(
            "collection root {} for `{source}` is not a directory",
            configured_root.unwrap_or(".")
        ));
    }
    Ok(root)
}

fn validate_collection_member(member: &str) -> Result<&Path, String> {
    let path = Path::new(member);
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(format!(
            "skills.lock collection member `{member}` must be a direct child directory name"
        ));
    }
    Ok(path)
}

fn validate_repository_path(path: &str) -> Result<&Path, String> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "Git skill path `{}` must stay within the repository",
            path.display()
        ));
    }
    Ok(path)
}

fn validate_git_skill_path(
    checkout: &Path,
    skill_path: &Path,
    manifest_path: &str,
) -> Result<(), String> {
    let repository = fs::canonicalize(checkout).map_err(|error| {
        format!(
            "failed to inspect Git checkout {}: {error}",
            checkout.display()
        )
    })?;
    let canonical_skill = fs::canonicalize(skill_path).map_err(|error| {
        format!("Git skill path `{manifest_path}` does not exist in the repository: {error}")
    })?;
    if !canonical_skill.starts_with(&repository) {
        return Err(format!(
            "Git skill path `{manifest_path}` escapes the repository"
        ));
    }
    let skill_document = canonical_skill.join("SKILL.md");
    let document_metadata = match fs::symlink_metadata(&skill_document) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "Git skill path `{manifest_path}` is missing SKILL.md"
            ));
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect Git skill document {}: {error}",
                skill_document.display()
            ));
        }
    };
    if !document_metadata.file_type().is_file() {
        return Err(format!(
            "Git skill path `{manifest_path}` must contain a regular SKILL.md file"
        ));
    }
    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path)
            .map_err(|error| format!("failed to remove {}: {error}", path.display())),
        Ok(_) => fs::remove_file(path)
            .map_err(|error| format!("failed to remove {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to inspect {}: {error}", path.display())),
    }
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir(destination).map_err(|error| {
        format!(
            "failed to create cached directory {}: {error}",
            destination.display()
        )
    })?;
    let entries = fs::read_dir(source)
        .map_err(|error| format!("failed to read local skill {}: {error}", source.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("failed to read {}: {error}", source.display()))?;
        let source_entry = entry.path();
        let destination_entry = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", source_entry.display()))?;
        if file_type.is_dir() {
            copy_directory(&source_entry, &destination_entry)?;
        } else if file_type.is_file() {
            fs::copy(&source_entry, &destination_entry)
                .map_err(|error| format!("failed to copy {}: {error}", source_entry.display()))?;
        } else {
            return Err(format!(
                "unsupported entry in local skill: {}",
                source_entry.display()
            ));
        }
    }
    Ok(())
}
