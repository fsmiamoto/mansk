use std::path::{Path, PathBuf};

pub fn resolve(name: &str, home: &Path) -> Result<PathBuf, String> {
    let relative = match name {
        "claude" => ".claude/skills",
        "agents" => ".agents/skills",
        _ => {
            return Err(format!(
                "unknown target `{name}`; supported targets are: claude, agents"
            ));
        }
    };
    Ok(home.join(relative))
}

pub fn validate_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Result<(), String> {
    for name in names {
        if !matches!(name, "claude" | "agents") {
            return Err(format!(
                "unknown target `{name}`; supported targets are: claude, agents"
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
    use std::path::Path;

    #[test]
    fn resolves_target_names_beneath_the_supplied_home() {
        let home = Path::new("/fake/home");
        assert_eq!(
            super::resolve("claude", home).unwrap(),
            home.join(".claude/skills")
        );
        assert_eq!(
            super::resolve("agents", home).unwrap(),
            home.join(".agents/skills")
        );
    }
}
