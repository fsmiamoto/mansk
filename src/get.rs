use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};

use dialoguer::{Confirm, MultiSelect, theme::ColorfulTheme};
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, value};

use crate::{
    apply, github, lock,
    manifest::{self, Manifest, Skill},
    output,
    plan::Action,
    resolve, targets,
};

#[derive(Debug, clap::Args)]
pub struct Options {
    /// GitHub repository, folder, SKILL.md URL, or owner/repo
    pub url: String,
    /// Select every discovered skill
    #[arg(long, conflicts_with = "skill")]
    pub all: bool,
    /// Select a repository-relative skill directory (repeatable; use . for the root)
    #[arg(long, value_name = "PATH")]
    pub skill: Vec<String>,
}

pub fn run(manifest_path: &Path, options: &Options, verbose: bool) -> Result<(), String> {
    let mut request = github::Request::parse(&options.url)?;
    let original = read_optional(manifest_path)?;
    let mut document = match original.as_deref() {
        Some(bytes) => std::str::from_utf8(bytes)
            .map_err(|error| format!("manifest is not UTF-8: {error}"))?
            .parse::<DocumentMut>()
            .map_err(|error| format!("failed to parse manifest: {error}"))?,
        None => {
            require_terminal(
                "No manifest found; create skills.toml with default-targets and [targets] first",
            )?;
            if !Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt(format!("Create {}?", manifest_path.display()))
                .default(true)
                .interact()
                .map_err(prompt_error)?
            {
                println!("Cancelled; no changes applied.");
                return Ok(());
            }
            "schema = 1\n"
                .parse::<DocumentMut>()
                .map_err(|error| error.to_string())?
        }
    };
    let mut existing = manifest::parse(&document.to_string(), manifest_path)?;
    if existing.default_targets.is_empty() {
        if !choose_targets(&mut document, &existing)? {
            println!("Cancelled; no changes applied.");
            return Ok(());
        }
        existing = manifest::parse(&document.to_string(), manifest_path)?;
    }
    targets::validate_names(
        existing.default_targets.iter().map(String::as_str),
        &existing.targets,
    )?;
    let cache = resolve::cache_home_from_env()?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let target_paths = targets::resolve(&existing.targets, home.as_deref(), &cache)?;
    let lock_path = lock::path_for_manifest(manifest_path);
    let original_lock = read_optional(&lock_path)?;
    let mut new_lock = if original_lock.is_some() {
        let locked = lock::read(manifest_path)?;
        locked
            .covers(&existing)
            .map_err(|error| format!("{error}; run `mansk update` first"))?;
        locked
    } else {
        if !existing.skills.is_empty() || !existing.collections.is_empty() {
            return Err(
                "Existing skills are not locked; run `mansk update` before `mansk get`".into(),
            );
        }
        lock::Lockfile::for_manifest(&existing, BTreeMap::new(), Vec::new())
    };

    // Retain the existing source spelling so its cache path and lock key stay stable.
    let mut pin: Option<(String, String)> = None;
    for (source, selector) in existing
        .skills
        .iter()
        .filter_map(|skill| Some((skill.source.as_deref()?, skill.selector.as_deref()?)))
        .chain(
            existing
                .collections
                .iter()
                .map(|collection| (collection.source.as_str(), collection.selector.as_str())),
        )
    {
        if github::canonical_source(source) != github::canonical_source(&request.source) {
            continue;
        }
        let mut commits = new_lock.git.get(source).into_iter().chain(
            new_lock
                .collections
                .iter()
                .filter(|collection| collection.source == source)
                .map(|collection| &collection.commit),
        );
        let commit = commits
            .next()
            .ok_or_else(|| "Missing repository lock; run `mansk update` first".to_owned())?;
        if commits.any(|other| other != commit)
            || pin.as_ref().is_some_and(|(_, old)| old != commit)
        {
            return Err(
                "Repository has conflicting locked commits; run `mansk update` first".into(),
            );
        }
        request.source = source.to_owned();
        pin.get_or_insert_with(|| (selector.to_owned(), commit.clone()));
    }
    println!("Discovering skills in {}…", request.source);
    let discovery = request.discover(
        &cache,
        pin.as_ref()
            .map(|(selector, commit)| (selector.as_str(), commit.as_str())),
    )?;
    if discovery.skills.is_empty() {
        return Err("No SKILL.md files found at that location".into());
    }
    let selected = select_skills(&discovery.skills, options)?;
    if selected.is_empty() {
        println!("Cancelled; no changes applied.");
        return Ok(());
    }
    let mut names = HashSet::new();
    for skill in &existing.skills {
        let name = match &skill.source {
            Some(source) => resolve::git_skill_name(&skill.path, source)?,
            None => Path::new(&skill.path)
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("Invalid skill path `{}`", skill.path))?
                .to_owned(),
        };
        names.insert(name);
    }
    names.extend(
        new_lock
            .collections
            .iter()
            .flat_map(|collection| collection.members.iter().cloned()),
    );
    let mut additions = Vec::new();
    for path in selected {
        if already_declared(&existing, &new_lock, &request.source, &path) {
            println!("Already added: {path}");
            continue;
        }
        let name = resolve::git_skill_name(&path, &request.source)?;
        if !names.insert(name.clone()) {
            return Err(format!(
                "duplicate skill name `{name}` (selected path `{path}`); no changes applied"
            ));
        }
        additions.push(Skill {
            source: Some(request.source.clone()),
            path,
            selector: Some(discovery.selector.clone()),
            targets: None,
        });
    }
    if additions.is_empty() {
        println!("All selected skills are already in the manifest.");
        return Ok(());
    }
    append_skills(&mut document, &additions)?;
    let updated_text = document.to_string();
    manifest::parse(&updated_text, manifest_path)?;
    let added = Manifest {
        schema: 1,
        default_targets: existing.default_targets,
        targets: existing.targets,
        skills: additions,
        collections: Vec::new(),
    };
    new_lock.git.insert(request.source, discovery.commit);
    let resolved = resolve::git_skills(&added, &new_lock.git, &[], &cache, false)?;
    // get only acts on selected destinations; stale installations belong to sync/update.
    let destinations: HashSet<_> = resolved
        .iter()
        .flat_map(|skill| {
            skill
                .targets
                .iter()
                .filter_map(|target| target_paths.get(target).map(|root| root.join(&skill.name)))
        })
        .collect();
    let actions: Vec<_> = crate::make_plan(&resolved, &target_paths, &cache)?
        .into_iter()
        .filter(|action| match action {
            Action::Remove { path } => destinations.contains(path),
            _ => true,
        })
        .collect();
    let summary = output::Summary::new(&actions, &target_paths, &HashSet::new());
    summary.print_plan("get", false, true);
    if verbose {
        crate::print_actions(&actions);
    }
    if read_optional(manifest_path)? != original || read_optional(&lock_path)? != original_lock {
        return Err("Manifest or lock changed during selection; rerun `mansk get`".into());
    }
    let lock_text = format!(
        "{}\n",
        serde_json::to_string_pretty(&new_lock).map_err(|error| error.to_string())?
    );
    // Prepare both writes before touching installed content, catching unwritable destinations early.
    let manifest_write = prepare_write(manifest_path, updated_text.as_bytes())?;
    let lock_write = prepare_write(&lock_path, lock_text.as_bytes())?;
    resolve::git_skills(&added, &new_lock.git, &[], &cache, true)?;
    let snapshots: Vec<_> = actions
        .iter()
        .filter_map(|action| match action {
            Action::Link { to, .. } => Some(to),
            Action::Remove { path } => Some(path),
            Action::Noop { .. } => None,
        })
        .collect::<HashSet<_>>()
        .into_iter()
        .map(|path| (path.clone(), fs::read_link(path).ok()))
        .collect();
    commit_write(manifest_write)?;
    let result = commit_write(lock_write).and_then(|()| apply::apply(&actions));
    if let Err(error) = result {
        let rollback = collect_errors([
            restore(manifest_path, original.as_deref()),
            restore(&lock_path, original_lock.as_deref()),
            restore_links(&snapshots),
        ]);
        return Err(match rollback {
            Ok(()) => format!("{error}; manifest, lock, and links restored"),
            Err(rollback) => format!("{error}; rollback also failed: {rollback}"),
        });
    }
    println!(
        "Saved {} skill(s) to {}.",
        added.skills.len(),
        manifest_path.display()
    );
    summary.print_success(true);
    Ok(())
}

fn select_skills(found: &[String], options: &Options) -> Result<Vec<String>, String> {
    if options.all {
        return Ok(found.to_vec());
    }
    if !options.skill.is_empty() {
        let mut selected = Vec::new();
        for requested in &options.skill {
            let path = found
                .iter()
                .find(|path| same_path(Path::new(path), Path::new(requested)))
                .ok_or_else(|| {
                    format!(
                        "Skill `{requested}` was not found; available paths: {}",
                        found.join(", ")
                    )
                })?;
            if !selected.contains(path) {
                selected.push(path.clone());
            }
        }
        return Ok(selected);
    }
    if found.len() == 1 {
        return Ok(found.to_vec());
    }
    require_terminal(&format!(
        "Found {} skills; use --all or repeat --skill PATH without a terminal. Available paths: {}",
        found.len(),
        found.join(", ")
    ))?;
    Ok(
        pick("Select skills (Space to toggle, Enter to install)", found)?
            .iter()
            .map(|&index| found[index].clone())
            .collect(),
    )
}

fn pick(prompt: &str, items: &[String]) -> Result<Vec<usize>, String> {
    MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .items(items)
        .interact_opt()
        .map(|selection| selection.unwrap_or_default())
        .map_err(prompt_error)
}

fn choose_targets(document: &mut DocumentMut, existing: &Manifest) -> Result<bool, String> {
    require_terminal(
        "No default targets configured; set default-targets and [targets] in the manifest first",
    )?;
    let choices: Vec<(String, String)> = if existing.targets.is_empty() {
        [
            ("claude", ".claude/skills"),
            ("codex", ".codex/skills"),
            ("pi", ".pi/agent/skills"),
        ]
        .into_iter()
        .map(|(name, path)| (name.to_owned(), path.to_owned()))
        .collect()
    } else {
        existing
            .targets
            .iter()
            .map(|(name, path)| (name.clone(), path.clone()))
            .collect()
    };
    let labels = choices
        .iter()
        .map(|(name, path)| format!("{name}  (~/{path})"))
        .collect::<Vec<_>>();
    let selected = pick("Select default target agents", &labels)?;
    if selected.is_empty() {
        return Ok(false);
    }
    let mut defaults = Array::new();
    for index in selected {
        let (name, path) = &choices[index];
        defaults.push(name.as_str());
        if existing.targets.is_empty() {
            document["targets"][name] = value(path);
        }
    }
    document["default-targets"] = value(defaults);
    Ok(true)
}

fn already_declared(
    existing: &Manifest,
    locked: &lock::Lockfile,
    source: &str,
    path: &str,
) -> bool {
    let canonical = github::canonical_source(source);
    existing.skills.iter().any(|skill| {
        skill
            .source
            .as_deref()
            .is_some_and(|source| github::canonical_source(source) == canonical)
            && same_path(Path::new(&skill.path), Path::new(path))
    }) || locked.collections.iter().any(|collection| {
        github::canonical_source(&collection.source) == canonical
            && collection.members.iter().any(|member| {
                same_path(
                    &Path::new(collection.root.as_deref().unwrap_or(".")).join(member),
                    Path::new(path),
                )
            })
    })
}

fn same_path(left: &Path, right: &Path) -> bool {
    let meaningful = |part: &std::path::Component<'_>| *part != std::path::Component::CurDir;
    left.components()
        .filter(meaningful)
        .eq(right.components().filter(meaningful))
}

fn append_skills(document: &mut DocumentMut, skills: &[Skill]) -> Result<(), String> {
    for skill in skills {
        let fields = [
            (
                "source",
                skill
                    .source
                    .as_deref()
                    .ok_or("Git skill is missing its source")?,
            ),
            (
                "selector",
                skill
                    .selector
                    .as_deref()
                    .ok_or("Git skill is missing its selector")?,
            ),
            ("path", skill.path.as_str()),
        ];
        if let Some(array) = document.get_mut("skills").and_then(Item::as_array_mut) {
            let mut table = toml_edit::InlineTable::new();
            for (key, text) in fields {
                table.insert(key, text.into());
            }
            array.push(table);
        } else {
            if document.get("skills").is_none() {
                document["skills"] = Item::ArrayOfTables(ArrayOfTables::new());
            }
            let mut table = Table::new();
            for (key, text) in fields {
                table[key] = value(text);
            }
            document["skills"]
                .as_array_of_tables_mut()
                .ok_or("skills must be an array of tables")?
                .push(table);
        }
    }
    Ok(())
}

fn require_terminal(message: &str) -> Result<(), String> {
    if io::stdin().is_terminal() && io::stderr().is_terminal() {
        Ok(())
    } else {
        Err(message.to_owned())
    }
}

fn prompt_error(error: dialoguer::Error) -> String {
    format!("Selection failed: {error}")
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("failed to read {}: {error}", path.display())),
    }
}

fn prepare_write(
    path: &Path,
    contents: &[u8],
) -> Result<(tempfile::NamedTempFile, PathBuf), String> {
    let destination = if path.exists() {
        fs::canonicalize(path).map_err(|error| error.to_string())?
    } else {
        path.to_owned()
    };
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    if let Ok(metadata) = fs::metadata(&destination) {
        file.as_file()
            .set_permissions(metadata.permissions())
            .map_err(|error| error.to_string())?;
    }
    file.write_all(contents)
        .map_err(|error| error.to_string())?;
    Ok((file, destination))
}

fn commit_write((file, path): (tempfile::NamedTempFile, PathBuf)) -> Result<(), String> {
    file.persist(&path)
        .map(|_| ())
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn restore(path: &Path, previous: Option<&[u8]>) -> Result<(), String> {
    match previous {
        Some(bytes) => commit_write(prepare_write(path, bytes)?),
        None => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        },
    }
}

fn restore_links(snapshots: &[(PathBuf, Option<PathBuf>)]) -> Result<(), String> {
    collect_errors(snapshots.iter().map(|(path, old)| {
        if fs::symlink_metadata(path).is_ok() {
            fs::remove_file(path).map_err(|error| error.to_string())?;
        }
        if let Some(from) = old {
            apply::apply(&[Action::Link {
                from: from.clone(),
                to: path.clone(),
            }])?;
        }
        Ok(())
    }))
}

fn collect_errors(results: impl IntoIterator<Item = Result<(), String>>) -> Result<(), String> {
    let errors: Vec<_> = results.into_iter().filter_map(Result::err).collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn rollback_continues_restoring_links_after_one_destination_fails() {
        let temp = tempfile::tempdir().unwrap();
        let blocked = temp.path().join("blocked");
        fs::create_dir(&blocked).unwrap();
        let removable = temp.path().join("removable");
        std::os::unix::fs::symlink("missing", &removable).unwrap();
        let restored = temp.path().join("restored");
        std::os::unix::fs::symlink("new", &restored).unwrap();
        let result = restore_links(&[
            (blocked.clone(), None),
            (removable.clone(), None),
            (restored.clone(), Some(PathBuf::from("original"))),
        ]);
        assert!(result.is_err());
        assert!(blocked.is_dir());
        assert!(fs::symlink_metadata(removable).is_err());
        assert_eq!(fs::read_link(restored).unwrap(), Path::new("original"));
    }
}
