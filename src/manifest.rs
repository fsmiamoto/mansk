use std::{collections::BTreeMap, fs, path::Path};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Manifest {
    pub schema: u32,
    #[serde(default)]
    pub default_targets: Vec<String>,
    #[serde(default)]
    pub targets: BTreeMap<String, String>,
    #[serde(default)]
    pub skills: Vec<Skill>,
    #[serde(default)]
    pub collections: Vec<Collection>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skill {
    pub source: Option<String>,
    #[allow(dead_code)] // Consumed by the resolution stage.
    pub path: String,
    pub selector: Option<String>,
    pub targets: Option<Vec<String>>,
}

#[allow(dead_code)] // Consumed by the resolution stage.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Collection {
    pub source: String,
    pub selector: String,
    pub root: Option<String>,
}

pub fn load(path: &Path) -> Result<Manifest, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("failed to read manifest {}: {error}", path.display()))?;
    let manifest: Manifest = toml::from_str(&contents)
        .map_err(|error| format!("failed to parse manifest {}: {error}", path.display()))?;
    if manifest.schema != 1 {
        return Err(format!(
            "unsupported schema version {}; expected 1 in {}",
            manifest.schema,
            path.display()
        ));
    }
    for (index, skill) in manifest.skills.iter().enumerate() {
        if skill.source.is_none() && skill.selector.is_some() {
            return Err(format!(
                "invalid skill {} in {}: local skill must not specify `selector`",
                index + 1,
                path.display()
            ));
        }
        if skill.source.is_some() && skill.selector.is_none() {
            return Err(format!(
                "invalid skill {} in {}: Git skill must specify `selector`",
                index + 1,
                path.display()
            ));
        }
    }
    Ok(manifest)
}
