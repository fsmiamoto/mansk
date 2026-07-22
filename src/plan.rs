use std::{
    collections::{HashMap, HashSet},
    fmt,
    path::PathBuf,
};

use crate::resolve::ResolvedSkill;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservedEntry {
    Symlink(PathBuf),
    Unmanaged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Link { from: PathBuf, to: PathBuf },
    Remove { path: PathBuf },
    Noop { path: PathBuf },
}

impl fmt::Display for Action {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Link { from, to } => {
                write!(formatter, "Link {} -> {}", to.display(), from.display())
            }
            Self::Remove { path } => write!(formatter, "Remove {}", path.display()),
            Self::Noop { path } => write!(formatter, "Noop {}", path.display()),
        }
    }
}

pub fn build(
    skills: &[ResolvedSkill],
    target_paths: &HashMap<String, PathBuf>,
    observed: &HashMap<PathBuf, ObservedEntry>,
) -> Result<Vec<Action>, String> {
    let mut skill_names = HashSet::new();
    for skill in skills {
        if !skill_names.insert(&skill.name) {
            return Err(format!("duplicate skill name `{}`", skill.name));
        }
    }

    let mut actions = Vec::new();
    for skill in skills {
        let mut planned_targets = HashSet::new();
        for target_name in &skill.targets {
            if !planned_targets.insert(target_name) {
                continue;
            }
            let target = target_paths
                .get(target_name)
                .ok_or_else(|| format!("unknown target `{target_name}`"))?;
            let to = target.join(&skill.name);
            match observed.get(&to) {
                None => actions.push(Action::Link {
                    from: skill.path.clone(),
                    to,
                }),
                Some(ObservedEntry::Symlink(from)) if from == &skill.path => {
                    actions.push(Action::Noop { path: to });
                }
                Some(ObservedEntry::Symlink(_)) => {
                    actions.push(Action::Remove { path: to.clone() });
                    actions.push(Action::Link {
                        from: skill.path.clone(),
                        to,
                    });
                }
                Some(_) => {
                    return Err(format!(
                        "target {} already exists and is not the requested mansk link; remove it manually",
                        to.display()
                    ));
                }
            }
        }
    }

    let mut stale: Vec<_> = observed
        .iter()
        .filter(|(path, entry)| {
            matches!(entry, ObservedEntry::Symlink(_))
                && !skills.iter().any(|skill| {
                    skill.targets.iter().any(|target_name| {
                        target_paths
                            .get(target_name)
                            .is_some_and(|target| target.join(&skill.name) == path.as_path())
                    })
                })
        })
        .map(|(path, _)| path.clone())
        .collect();
    stale.sort();
    actions.extend(stale.into_iter().map(|path| Action::Remove { path }));
    Ok(actions)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review_skill() -> ResolvedSkill {
        ResolvedSkill {
            name: "review".into(),
            path: "/cache/local/review".into(),
            targets: vec!["primary".into()],
        }
    }

    fn target_paths() -> HashMap<String, PathBuf> {
        HashMap::from([("primary".into(), PathBuf::from("/home/.primary/skills"))])
    }

    #[test]
    fn fresh_install() {
        assert_eq!(
            build(&[review_skill()], &target_paths(), &HashMap::new()).unwrap(),
            vec![Action::Link {
                from: "/cache/local/review".into(),
                to: "/home/.primary/skills/review".into(),
            }]
        );
    }

    #[test]
    fn noop_rerun() {
        let observed = HashMap::from([(
            PathBuf::from("/home/.primary/skills/review"),
            ObservedEntry::Symlink("/cache/local/review".into()),
        )]);
        assert_eq!(
            build(&[review_skill()], &target_paths(), &observed).unwrap(),
            vec![Action::Noop {
                path: "/home/.primary/skills/review".into(),
            }]
        );
    }

    #[test]
    fn duplicate_skill_names_are_rejected_before_planning_overlapping_targets() {
        let mut duplicate = review_skill();
        duplicate.path = "/cache/other/review".into();
        duplicate.targets = vec!["primary".into(), "secondary".into()];
        let targets = HashMap::from([
            ("primary".into(), PathBuf::from("/shared/skills")),
            ("secondary".into(), PathBuf::from("/shared/skills")),
        ]);

        let error = build(&[review_skill(), duplicate], &targets, &HashMap::new()).unwrap_err();

        assert_eq!(error, "duplicate skill name `review`");
    }

    #[test]
    fn stale_unmanaged_entries_are_ignored() {
        let observed = HashMap::from([
            (
                PathBuf::from("/home/.primary/skills/real-directory"),
                ObservedEntry::Unmanaged,
            ),
            (
                PathBuf::from("/home/.primary/skills/external-link"),
                ObservedEntry::Unmanaged,
            ),
        ]);

        assert_eq!(build(&[], &target_paths(), &observed).unwrap(), vec![]);
    }

    #[test]
    fn unmanaged_collision_is_rejected_with_manual_removal_instructions() {
        let path = PathBuf::from("/home/.primary/skills/review");
        let observed = HashMap::from([(path.clone(), ObservedEntry::Unmanaged)]);

        let error = build(&[review_skill()], &target_paths(), &observed).unwrap_err();

        assert!(error.contains(path.to_str().unwrap()));
        assert!(error.contains("remove it manually"));
    }

    #[test]
    fn changed_owned_link_is_replaced() {
        let path = PathBuf::from("/home/.primary/skills/review");
        let observed = HashMap::from([(
            path.clone(),
            ObservedEntry::Symlink("/cache/old/review".into()),
        )]);

        assert_eq!(
            build(&[review_skill()], &target_paths(), &observed).unwrap(),
            vec![
                Action::Remove { path: path.clone() },
                Action::Link {
                    from: "/cache/local/review".into(),
                    to: path,
                },
            ]
        );
    }

    #[test]
    fn manifest_removal_prunes_an_owned_link() {
        let path = PathBuf::from("/home/.primary/skills/review");
        let observed = HashMap::from([(
            path.clone(),
            ObservedEntry::Symlink("/cache/local/review".into()),
        )]);

        assert_eq!(
            build(&[], &target_paths(), &observed).unwrap(),
            vec![Action::Remove { path }]
        );
    }
}
