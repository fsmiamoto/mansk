use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
};

use clap::{Parser, Subcommand};

mod apply;
mod lock;
mod manifest;
mod plan;
mod resolve;
mod targets;

#[derive(Debug, Parser)]
#[command(name = "mansk", about = "Reproducible agent skills manager")]
struct Cli {
    /// Path to the skills manifest
    #[arg(long, global = true, value_name = "PATH")]
    manifest: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Install exactly the skills recorded in the lockfile
    Sync {
        /// Print the plan without changing target directories
        #[arg(long)]
        dry_run: bool,
    },
    /// Resolve manifest selectors and update the lockfile
    Update {
        /// Print the plan without changing target directories
        #[arg(long)]
        dry_run: bool,
        /// Apply without prompting for confirmation
        #[arg(long)]
        yes: bool,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    let manifest_path = match cli.manifest {
        Some(path) => path,
        None => targets::manifest_path_from_env()?,
    };
    let manifest = manifest::load(&manifest_path)?;
    let target_names: Vec<&str> = manifest
        .default_targets
        .iter()
        .chain(
            manifest
                .skills
                .iter()
                .filter_map(|skill| skill.targets.as_ref())
                .flatten(),
        )
        .map(String::as_str)
        .collect();
    targets::validate_names(target_names.iter().copied())?;

    if manifest.skills.is_empty()
        && manifest.collections.is_empty()
        && !lock::path_for_manifest(&manifest_path).exists()
    {
        return Ok(());
    }

    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "cannot resolve targets: HOME is not set".to_owned())?;
    let target_paths = target_paths(&home)?;
    let cache_root = resolve::cache_home_from_env()?;

    match cli.command {
        Command::Sync { dry_run } => {
            let lockfile = lock::read(&manifest_path)?;
            lockfile.covers(&manifest)?;
            let mut resolved =
                resolve::local_skills(&manifest, &manifest_path, &cache_root, !dry_run)?;
            resolved.extend(resolve::git_skills(
                &manifest,
                &lockfile.git,
                &lockfile.collections,
                &cache_root,
                !dry_run,
            )?);
            let actions = make_plan(&resolved, &target_paths, &cache_root)?;
            print_actions(&actions);
            if !dry_run {
                apply::apply(&actions)?;
            }
        }
        Command::Update { dry_run, yes } => {
            let commits = resolve::resolve_git_selectors(&manifest)?;
            let old_lock = if lock::path_for_manifest(&manifest_path).exists() {
                Some(lock::read(&manifest_path)?)
            } else {
                None
            };
            let collections = resolve::discover_collections(&manifest, &commits, &cache_root)?;
            print_commit_changes(old_lock.as_ref(), &commits);
            print_member_changes(old_lock.as_ref(), &collections);
            let explicit_commits = commits
                .iter()
                .filter(|(source, _)| {
                    manifest
                        .skills
                        .iter()
                        .any(|skill| skill.source.as_ref() == Some(source))
                })
                .map(|(source, commit)| (source.clone(), commit.clone()))
                .collect();
            let new_lock =
                lock::Lockfile::for_manifest(&manifest, explicit_commits, collections.clone());
            let mut resolved =
                resolve::local_skills(&manifest, &manifest_path, &cache_root, false)?;
            resolved.extend(resolve::git_skills(
                &manifest,
                &commits,
                &collections,
                &cache_root,
                false,
            )?);
            let actions = make_plan(&resolved, &target_paths, &cache_root)?;
            print_actions(&actions);
            if dry_run {
                return Ok(());
            }
            if !yes && !confirm_update()? {
                println!("Declined");
                return Ok(());
            }
            resolve::local_skills(&manifest, &manifest_path, &cache_root, true)?;
            resolve::git_skills(&manifest, &commits, &collections, &cache_root, true)?;
            lock::write(&manifest_path, &new_lock)?;
            apply::apply(&actions)?;
        }
    }
    Ok(())
}

fn target_paths(home: &Path) -> Result<HashMap<String, PathBuf>, String> {
    ["claude", "agents"]
        .into_iter()
        .map(|name| targets::resolve(name, home).map(|path| (name.to_owned(), path)))
        .collect()
}

fn make_plan(
    skills: &[resolve::ResolvedSkill],
    target_paths: &HashMap<String, PathBuf>,
    cache_root: &Path,
) -> Result<Vec<plan::Action>, String> {
    let canonical_cache = fs::canonicalize(cache_root).ok();
    let mut observed = HashMap::new();
    for target in target_paths.values() {
        let entries = match fs::read_dir(target) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "failed to read target directory {}: {error}",
                    target.display()
                ));
            }
        };
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!(
                    "failed to read target directory {}: {error}",
                    target.display()
                )
            })?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("failed to inspect target {}: {error}", path.display()))?;
            let observed_entry = if metadata.file_type().is_symlink()
                && canonical_cache.as_ref().is_some_and(|cache| {
                    fs::canonicalize(&path).is_ok_and(|destination| destination.starts_with(cache))
                }) {
                let destination = fs::read_link(&path).map_err(|error| {
                    format!("failed to read target link {}: {error}", path.display())
                })?;
                plan::ObservedEntry::Symlink(destination)
            } else {
                plan::ObservedEntry::Unmanaged
            };
            observed.insert(path, observed_entry);
        }
    }
    plan::build(skills, target_paths, &observed)
}

fn print_actions(actions: &[plan::Action]) {
    for action in actions {
        println!("{action}");
    }
}

fn print_commit_changes(
    old_lock: Option<&lock::Lockfile>,
    commits: &std::collections::BTreeMap<String, String>,
) {
    let mut old_commits = std::collections::BTreeMap::new();
    if let Some(lock) = old_lock {
        old_commits.extend(lock.git.iter().map(|(source, commit)| (source, commit)));
        old_commits.extend(
            lock.collections
                .iter()
                .map(|collection| (&collection.source, &collection.commit)),
        );
    }

    for (source, commit) in commits {
        let old = old_commits.get(source);
        if old.is_some_and(|old| *old == commit) {
            continue;
        }
        let old = old.map_or("(new)", |commit| abbreviated_commit(commit));
        println!("{source}: {old} → {}", abbreviated_commit(commit));
    }
    for (source, commit) in old_commits {
        if !commits.contains_key(source) {
            println!("{source}: {} → (removed)", abbreviated_commit(commit));
        }
    }
}

fn print_member_changes(old_lock: Option<&lock::Lockfile>, collections: &[lock::LockedCollection]) {
    let old_collections = old_lock
        .map(|lock| lock.collections.as_slice())
        .unwrap_or(&[]);
    for collection in collections {
        let old_members = old_collections
            .iter()
            .find(|old| old.source == collection.source && old.root == collection.root)
            .map(|old| old.members.as_slice())
            .unwrap_or(&[]);
        print_member_diff(&collection.source, old_members, &collection.members);
    }
    for old in old_collections {
        if !collections
            .iter()
            .any(|new| new.source == old.source && new.root == old.root)
        {
            print_member_diff(&old.source, &old.members, &[]);
        }
    }
}

fn print_member_diff(source: &str, old_members: &[String], new_members: &[String]) {
    let added: Vec<_> = new_members
        .iter()
        .filter(|member| !old_members.contains(member))
        .cloned()
        .collect();
    let removed: Vec<_> = old_members
        .iter()
        .filter(|member| !new_members.contains(member))
        .cloned()
        .collect();
    if !added.is_empty() {
        println!("{source}: members added: {}", added.join(", "));
    }
    if !removed.is_empty() {
        println!("{source}: members removed: {}", removed.join(", "));
    }
}

fn abbreviated_commit(commit: &str) -> &str {
    &commit[..commit.len().min(7)]
}

fn confirm_update() -> Result<bool, String> {
    eprint!("Apply update? [y/N] ");
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| format!("failed to read confirmation: {error}"))?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
