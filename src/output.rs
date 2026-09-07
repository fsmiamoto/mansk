use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::PathBuf,
};

use crate::plan::Action;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Change {
    Update,
    Add,
    Remove,
}

impl Change {
    fn label(self) -> &'static str {
        match self {
            Self::Update => "Update",
            Self::Add => "Add",
            Self::Remove => "Remove",
        }
    }
}

#[derive(Debug)]
struct Row {
    change: Change,
    skill: String,
    targets: BTreeSet<String>,
}

#[derive(Default)]
struct Destination {
    link: bool,
    remove: bool,
    noop: bool,
}

/// A user-facing summary of installation changes, independent of action order.
pub struct Summary {
    rows: Vec<Row>,
    changed_skills: usize,
    unchanged_skills: usize,
    total_skills: usize,
    active_targets: usize,
    updates: usize,
    additions: usize,
    removals: usize,
}

impl Summary {
    pub fn new(
        actions: &[Action],
        targets: &HashMap<String, PathBuf>,
        refreshed_names: &HashSet<String>,
    ) -> Self {
        let mut destinations = BTreeMap::<&PathBuf, Destination>::new();
        for action in actions {
            match action {
                Action::Link { to, .. } => destinations.entry(to).or_default().link = true,
                Action::Remove { path } => destinations.entry(path).or_default().remove = true,
                Action::Noop { path } => destinations.entry(path).or_default().noop = true,
            }
        }

        let mut grouped = BTreeMap::<(Change, String), BTreeSet<String>>::new();
        let mut changed = BTreeSet::new();
        let mut unchanged = BTreeSet::new();
        let mut installed = BTreeSet::new();
        let mut active_targets = BTreeSet::new();
        let (mut updates, mut additions, mut removals) = (0, 0, 0);
        for (path, destination) in destinations {
            let skill = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "(unnamed skill)".into());
            let names: BTreeSet<_> = targets
                .iter()
                .filter(|(_, root)| Some(root.as_path()) == path.parent())
                .map(|(name, _)| name.clone())
                .collect();
            let change = match (destination.link, destination.remove) {
                (true, true) => {
                    updates += 1;
                    Some(Change::Update)
                }
                (true, false) => {
                    additions += 1;
                    Some(Change::Add)
                }
                (false, true) => {
                    removals += 1;
                    Some(Change::Remove)
                }
                (false, false) if destination.noop && refreshed_names.contains(&skill) => {
                    updates += 1;
                    Some(Change::Update)
                }
                (false, false) => None,
            };
            if destination.link || (destination.noop && !destination.remove) {
                installed.insert(skill.clone());
                active_targets.extend(names.iter().cloned());
            }
            if let Some(change) = change {
                changed.insert(skill.clone());
                let row_targets = grouped.entry((change, skill)).or_default();
                if names.is_empty() {
                    row_targets.insert("unknown agent".into());
                } else {
                    row_targets.extend(names);
                }
            } else if destination.noop {
                unchanged.insert(skill);
            }
        }
        let unchanged_skills = unchanged.difference(&changed).count();
        Self {
            rows: grouped
                .into_iter()
                .map(|((change, skill), targets)| Row {
                    change,
                    skill,
                    targets,
                })
                .collect(),
            changed_skills: changed.len(),
            unchanged_skills,
            total_skills: installed.len(),
            active_targets: active_targets.len(),
            updates,
            additions,
            removals,
        }
    }

    pub fn has_changes(&self) -> bool {
        !self.rows.is_empty()
    }

    pub fn print_plan(&self, command: &str, dry_run: bool, lock_changed: bool) {
        print!("{}", self.plan_text(command, dry_run, lock_changed));
    }

    fn plan_text(&self, command: &str, dry_run: bool, lock_changed: bool) -> String {
        let mut text = String::new();
        if self.has_changes() {
            text.push_str(&format!("mansk {command}\n\n"));
            let width = self
                .rows
                .iter()
                .map(|row| row.skill.chars().count())
                .max()
                .unwrap_or(0);
            for row in &self.rows {
                let targets = row.targets.iter().cloned().collect::<Vec<_>>().join(", ");
                text.push_str(&format!(
                    "  {:<6}  {:width$}  {targets}\n",
                    row.change.label(),
                    row.skill,
                ));
            }
            text.push_str(&format!(
                "\n{} changing · {} unchanged\n",
                counted(self.changed_skills, "skill"),
                self.unchanged_skills,
            ));
        } else if !lock_changed {
            text.push_str(&format!(
                "Everything is up to date · {} across {}\n",
                counted(self.total_skills, "skill"),
                counted(self.active_targets, "agent"),
            ));
        } else {
            text.push_str("No skill installation changes.\n");
        }
        if lock_changed {
            text.push_str("Lockfile will be updated.\n");
        }
        if dry_run {
            text.push_str("Preview only; no changes applied.\n");
        }
        text
    }

    pub fn print_success(&self, lock_changed: bool) {
        print!("{}", self.success_text(lock_changed));
    }

    fn success_text(&self, lock_changed: bool) -> String {
        let mut parts = Vec::new();
        for (count, verb) in [
            (self.updates, "updated"),
            (self.additions, "added"),
            (self.removals, "removed"),
        ] {
            if count > 0 {
                parts.push(format!("{verb} {}", counted(count, "installation")));
            }
        }
        if parts.is_empty() {
            return if lock_changed {
                "✓ Lockfile updated.\n".into()
            } else {
                String::new()
            };
        }
        let mut sentence = parts.join(", ");
        sentence.replace_range(..1, &sentence[..1].to_uppercase());
        let suffix = if lock_changed {
            " Lockfile updated."
        } else {
            ""
        };
        format!("✓ {sentence}.{suffix}\n")
    }
}

fn counted(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets() -> HashMap<String, PathBuf> {
        HashMap::from([
            ("pi".into(), "/targets/pi".into()),
            ("claude".into(), "/targets/claude".into()),
            ("unused".into(), "/targets/unused".into()),
        ])
    }

    fn link(agent: &str, skill: &str) -> Action {
        Action::Link {
            from: format!("/cache/{skill}").into(),
            to: format!("/targets/{agent}/{skill}").into(),
        }
    }

    fn remove(agent: &str, skill: &str) -> Action {
        Action::Remove {
            path: format!("/targets/{agent}/{skill}").into(),
        }
    }

    fn noop(agent: &str, skill: &str) -> Action {
        Action::Noop {
            path: format!("/targets/{agent}/{skill}").into(),
        }
    }

    #[test]
    fn replacements_group_by_skill_with_sorted_agents_independent_of_order() {
        let actions = vec![
            link("pi", "review"),
            remove("claude", "review"),
            remove("pi", "review"),
            link("claude", "review"),
        ];
        let summary = Summary::new(&actions, &targets(), &HashSet::new());
        assert_eq!(summary.rows.len(), 1);
        assert_eq!(summary.changed_skills, 1);
        assert_eq!(summary.total_skills, 1);
        assert_eq!(summary.active_targets, 2);
        assert_eq!(summary.updates, 2);
        assert_eq!(summary.additions, 0);
        assert_eq!(summary.removals, 0);
        assert_eq!(
            summary.plan_text("update", false, false),
            "mansk update\n\n  Update  review  claude, pi\n\n1 skill changing · 0 unchanged\n"
        );
        assert_eq!(summary.success_text(false), "✓ Updated 2 installations.\n");
    }

    #[test]
    fn mixed_target_states_count_each_changed_skill_once() {
        let summary = Summary::new(
            &[
                noop("claude", "review"),
                link("pi", "review"),
                noop("pi", "stable"),
                noop("claude", "stable"),
                remove("pi", "old"),
            ],
            &targets(),
            &HashSet::new(),
        );
        assert_eq!(summary.changed_skills, 2);
        assert_eq!(summary.unchanged_skills, 1);
        assert_eq!(summary.total_skills, 2);
        assert_eq!(summary.active_targets, 2);
        assert_eq!(
            summary.success_text(false),
            "✓ Added 1 installation, removed 1 installation.\n"
        );
        assert!(!summary.plan_text("sync", false, false).contains("/targets"));
    }

    #[test]
    fn same_skill_can_have_different_changes_per_agent() {
        let summary = Summary::new(
            &[remove("pi", "review"), link("claude", "review")],
            &targets(),
            &HashSet::new(),
        );
        assert_eq!(summary.rows.len(), 2);
        assert_eq!(summary.changed_skills, 1);
        assert_eq!(summary.total_skills, 1);
        assert_eq!(summary.active_targets, 1);
        assert_eq!(summary.rows[0].change, Change::Add);
        assert_eq!(summary.rows[1].change, Change::Remove);
    }

    #[test]
    fn removal_only_leaves_no_installed_skills_or_active_agents() {
        let summary = Summary::new(&[remove("pi", "old")], &targets(), &HashSet::new());
        assert_eq!(summary.total_skills, 0);
        assert_eq!(summary.active_targets, 0);
        assert_eq!(summary.changed_skills, 1);
        assert_eq!(summary.success_text(false), "✓ Removed 1 installation.\n");
    }

    #[test]
    fn unchanged_and_empty_runs_are_concise_and_pluralized() {
        let summary = Summary::new(&[noop("pi", "review")], &targets(), &HashSet::new());
        assert!(!summary.has_changes());
        assert_eq!(
            summary.plan_text("sync", false, false),
            "Everything is up to date · 1 skill across 1 agent\n"
        );
        let empty = Summary::new(&[], &targets(), &HashSet::new());
        assert_eq!(
            empty.plan_text("sync", true, false),
            "Everything is up to date · 0 skills across 0 agents\nPreview only; no changes applied.\n"
        );
    }

    #[test]
    fn refreshed_cache_counts_unchanged_links_as_updates() {
        let summary = Summary::new(
            &[
                noop("pi", "review"),
                noop("claude", "review"),
                noop("pi", "stable"),
            ],
            &targets(),
            &HashSet::from(["review".into()]),
        );
        assert_eq!(summary.rows.len(), 1);
        assert_eq!(summary.rows[0].change, Change::Update);
        assert_eq!(summary.updates, 2);
        assert_eq!(summary.changed_skills, 1);
        assert_eq!(summary.unchanged_skills, 1);
        assert_eq!(summary.total_skills, 2);
        assert_eq!(summary.active_targets, 2);
    }

    #[test]
    fn lock_only_update_does_not_claim_everything_is_up_to_date() {
        let summary = Summary::new(&[noop("pi", "review")], &targets(), &HashSet::new());
        assert_eq!(
            summary.plan_text("update", true, true),
            "No skill installation changes.\nLockfile will be updated.\nPreview only; no changes applied.\n"
        );
        assert_eq!(summary.success_text(true), "✓ Lockfile updated.\n");
        assert_eq!(summary.success_text(false), "");
    }
}
