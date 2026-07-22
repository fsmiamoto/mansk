use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Component, Path, PathBuf},
};

pub fn resolve(
    targets: &BTreeMap<String, String>,
    home: Option<&Path>,
    cache_root: &Path,
) -> Result<HashMap<String, PathBuf>, String> {
    let comparable_cache = resolve_for_comparison(cache_root);
    let mut resolved = HashMap::new();
    let mut names_by_path = HashMap::new();
    for (name, configured) in targets {
        if configured.is_empty() {
            return Err(format!("target `{name}` has an empty path"));
        }
        let configured = Path::new(configured);
        let path = if configured.is_absolute() {
            configured.to_owned()
        } else {
            home.ok_or_else(|| format!("cannot resolve relative target `{name}`: HOME is not set"))?
                .join(configured)
        };
        if let Some(symlink) = unresolved_symlink_ancestor(&path) {
            return Err(format!(
                "target `{name}` traverses unresolved symlink {}",
                symlink.display()
            ));
        }
        let comparison_path = resolve_for_comparison(&path);
        if comparison_path.starts_with(&comparable_cache) {
            return Err(format!(
                "target `{name}` resolves inside the mansk cache {}",
                cache_root.display()
            ));
        }
        if let Some(previous) = names_by_path.insert(comparison_path, name) {
            return Err(format!(
                "targets `{previous}` and `{name}` resolve to the same directory {}",
                path.display()
            ));
        }
        resolved.insert(name.clone(), path);
    }
    Ok(resolved)
}

fn unresolved_symlink_ancestor(path: &Path) -> Option<PathBuf> {
    let mut candidate = Some(path);
    while let Some(path) = candidate {
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if metadata.file_type().is_symlink() && fs::canonicalize(path).is_err() {
                return Some(path.to_owned());
            }
        }
        candidate = path.parent();
    }
    None
}

fn resolve_for_comparison(path: &Path) -> PathBuf {
    let mut ancestor = path;
    loop {
        if let Ok(canonical) = fs::canonicalize(ancestor) {
            if let Ok(remainder) = path.strip_prefix(ancestor) {
                return normalize_lexically(&canonical.join(remainder));
            }
        }
        match ancestor.parent() {
            Some(parent) if parent != ancestor => ancestor = parent,
            _ => return normalize_lexically(path),
        }
    }
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(
                    normalized.components().next_back(),
                    Some(Component::Normal(_))
                ) {
                    normalized.pop();
                } else if !normalized.has_root() {
                    normalized.push(component.as_os_str());
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

pub fn validate_names<'a>(
    names: impl IntoIterator<Item = &'a str>,
    targets: &BTreeMap<String, String>,
) -> Result<(), String> {
    for name in names {
        if !targets.contains_key(name) {
            return Err(format!(
                "unknown target `{name}`; declare it under [targets]"
            ));
        }
    }
    Ok(())
}

pub fn manifest_path_from_env() -> Result<PathBuf, String> {
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(config_home).join("mansk/skills.toml"));
    }

    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".config/mansk/skills.toml"))
        .ok_or_else(|| "cannot locate the manifest: neither XDG_CONFIG_HOME nor HOME is set".into())
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::Path};

    #[test]
    fn resolves_relative_paths_beneath_home_and_preserves_absolute_paths() {
        let home = Path::new("/fake/home");
        let targets = BTreeMap::from([
            ("relative".into(), ".tool/skills".into()),
            ("absolute".into(), "/shared/skills".into()),
        ]);

        let resolved = super::resolve(&targets, Some(home), Path::new("/cache/mansk")).unwrap();

        assert_eq!(resolved["relative"], home.join(".tool/skills"));
        assert_eq!(resolved["absolute"], Path::new("/shared/skills"));
    }
}
