use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::manifest::Manifest;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Lockfile {
    pub schema: u32,
    pub local: Vec<LockedLocal>,
    #[serde(default)]
    pub git: BTreeMap<String, String>,
    #[serde(default)]
    pub collections: Vec<LockedCollection>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LockedLocal {
    pub source: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LockedCollection {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    pub commit: String,
    pub members: Vec<String>,
}

impl Lockfile {
    pub fn for_manifest(
        manifest: &Manifest,
        git: BTreeMap<String, String>,
        collections: Vec<LockedCollection>,
    ) -> Self {
        Self {
            schema: 1,
            local: manifest
                .skills
                .iter()
                .filter(|skill| skill.source.is_none())
                .map(|skill| LockedLocal {
                    source: skill.path.clone(),
                })
                .collect(),
            git,
            collections,
        }
    }

    pub fn covers(&self, manifest: &Manifest) -> Result<(), String> {
        for skill in &manifest.skills {
            if let Some(source) = &skill.source {
                if !self.git.contains_key(source) {
                    return Err(format!("skills.lock does not cover Git source `{source}`"));
                }
            } else if !self.local.iter().any(|locked| locked.source == skill.path) {
                return Err(format!(
                    "skills.lock does not cover local source `{}`",
                    skill.path
                ));
            }
        }
        if self.collections.len() != manifest.collections.len() {
            return Err(format!(
                "skills.lock does not cover collections: expected {}, found {}",
                manifest.collections.len(),
                self.collections.len()
            ));
        }
        for (index, (collection, locked)) in manifest
            .collections
            .iter()
            .zip(&self.collections)
            .enumerate()
        {
            if collection.source != locked.source || collection.root != locked.root {
                return Err(format!(
                    "skills.lock does not cover collection {} from `{}` with root `{}`",
                    index + 1,
                    collection.source,
                    collection.root.as_deref().unwrap_or(".")
                ));
            }
        }
        Ok(())
    }
}

pub fn path_for_manifest(manifest_path: &Path) -> PathBuf {
    manifest_path.with_file_name("skills.lock")
}

pub fn read(manifest_path: &Path) -> Result<Lockfile, String> {
    let path = path_for_manifest(manifest_path);
    let contents = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read lockfile {}: {error}", path.display()))?;
    let lock: Lockfile = serde_json::from_str(&contents)
        .map_err(|error| format!("failed to parse lockfile {}: {error}", path.display()))?;
    if lock.schema != 1 {
        return Err(format!(
            "unsupported lockfile schema {} in {}",
            lock.schema,
            path.display()
        ));
    }
    Ok(lock)
}

pub fn write(manifest_path: &Path, lock: &Lockfile) -> Result<(), String> {
    let path = path_for_manifest(manifest_path);
    let contents = serde_json::to_string_pretty(lock)
        .map_err(|error| format!("failed to serialize lockfile: {error}"))?;
    fs::write(&path, format!("{contents}\n"))
        .map_err(|error| format!("failed to write lockfile {}: {error}", path.display()))
}
