//! Project discovery — walk upward to find `saturn.toml`.

use crate::manifest::Manifest;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Result of project discovery.
#[derive(Debug, Clone)]
pub struct DiscoveredProject {
    /// The project root directory (contains `saturn.toml`).
    pub root: PathBuf,
    /// The parsed manifest.
    pub manifest: Manifest,
    /// The source root directory.
    pub source_root: PathBuf,
    /// The resolved entry point path.
    pub entry: PathBuf,
}

/// Project discovery strategy.
pub struct ProjectDiscovery;

impl ProjectDiscovery {
    /// Discover a Saturnite project starting from the given path.
    ///
    /// Walks upward from `start` looking for `saturn.toml`. If `start` is a
    /// file, discovery begins from its parent directory. If no `saturn.toml`
    /// is found, synthesizes a minimal manifest from the directory name.
    ///
    /// This mirrors Cargo's `Cargo.toml` root-finding behavior.
    pub fn discover(start: &Path) -> Result<DiscoveredProject> {
        // If start is a file, begin from its parent.
        let start_dir = if start.is_file() {
            start.parent().unwrap_or_else(|| Path::new("."))
        } else {
            start
        };

        // Walk upward looking for saturn.toml.
        let mut current: &Path = start_dir;
        loop {
            let config_path = current.join("saturn.toml");
            if config_path.is_file() {
                let root = current.to_path_buf();
                let manifest = Manifest::from_dir(&root)?;
                let source_root = manifest.source_root(&root);
                let entry = manifest.resolve_entry(&root);
                return Ok(DiscoveredProject {
                    root,
                    manifest,
                    source_root,
                    entry,
                });
            }

            match current.parent() {
                Some(parent) => current = parent,
                None => {
                    // No saturn.toml found — synthesize one.
                    let name = start_dir
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("project")
                        .to_string();
                    let manifest = Manifest::from_name(&name)?;
                    let root = start_dir.to_path_buf();
                    let source_root = manifest.source_root(&root);
                    let entry = manifest.resolve_entry(&root);
                    return Ok(DiscoveredProject {
                        root,
                        manifest,
                        source_root,
                        entry,
                    });
                }
            }
        }
    }

    /// Discover a project from the current working directory.
    pub fn discover_from_cwd() -> Result<DiscoveredProject> {
        let cwd = std::env::current_dir()?;
        Self::discover(&cwd)
    }

    /// Check that the discovered entry point exists.
    pub fn ensure_entry_exists(project: &DiscoveredProject) -> Result<()> {
        if !project.entry.is_file() {
            anyhow::bail!(
                "no entry point found: expected {} \
                 (create a saturn.toml project with src/main.stn or pass a file explicitly)",
                project.entry.display()
            );
        }
        Ok(())
    }
}
