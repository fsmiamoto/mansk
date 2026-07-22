use std::fs;

use crate::plan::Action;

pub fn apply(actions: &[Action]) -> Result<(), String> {
    for action in actions {
        match action {
            Action::Link { from, to } => {
                let parent = to.parent().ok_or_else(|| {
                    format!("target link {} has no parent directory", to.display())
                })?;
                fs::create_dir_all(parent).map_err(|error| {
                    format!(
                        "failed to create target directory {}: {error}",
                        parent.display()
                    )
                })?;
                #[cfg(unix)]
                std::os::unix::fs::symlink(from, to).map_err(|error| {
                    format!(
                        "failed to link {} to {}: {error}",
                        to.display(),
                        from.display()
                    )
                })?;
                #[cfg(not(unix))]
                return Err("mansk MVP supports symlink installation on Unix only".into());
            }
            Action::Remove { path } => fs::remove_file(path).map_err(|error| {
                format!("failed to remove target link {}: {error}", path.display())
            })?,
            Action::Noop { .. } => {}
        }
    }
    Ok(())
}
