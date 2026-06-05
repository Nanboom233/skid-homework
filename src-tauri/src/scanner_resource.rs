use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ResourceRootCandidate {
    pub source: &'static str,
    pub path: PathBuf,
}

pub fn build_resource_root_candidates(
    resource_dir_hint: Option<PathBuf>,
    installed_assets_current_dir_hint: Option<PathBuf>,
) -> Vec<ResourceRootCandidate> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    let mut push_candidate = |source: &'static str, path: PathBuf| {
        if seen.insert(path.clone()) {
            candidates.push(ResourceRootCandidate { source, path });
        }
    };

    if let Some(installed_assets_current_dir) = installed_assets_current_dir_hint {
        push_candidate("installed-assets-current", installed_assets_current_dir);
    }

    if let Some(resource_dir) = resource_dir_hint {
        push_candidate("tauri-resource-dir", resource_dir);
    }

    candidates
}

pub fn select_resource_root(
    candidates: &[ResourceRootCandidate],
    interesting_paths: &[&str],
) -> Option<ResourceRootCandidate> {
    candidates
        .iter()
        .find(|candidate| score_resource_root(&candidate.path, interesting_paths) > 0)
        .cloned()
}

pub fn score_resource_root(root: &Path, interesting_paths: &[&str]) -> usize {
    interesting_paths
        .iter()
        .filter(|relative_path| root.join(relative_path).exists())
        .count()
}

pub fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub fn validate_path_containment(path: &Path, root: &Path) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("Failed to canonicalize path {:?}: {error}", path))?;
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("Failed to canonicalize root {:?}: {error}", root))?;

    if !canonical.starts_with(&canonical_root) {
        return Err(format!(
            "Path traversal detected: {:?} is not under {:?}.",
            canonical, canonical_root
        ));
    }

    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_counts_existing_interesting_paths() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let score = score_resource_root(&root, &["Cargo.toml", "missing-file"]);
        assert_eq!(score, 1);
    }

    #[test]
    fn select_resource_root_prefers_candidate_order() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        std::fs::write(first.path().join("one"), b"1").unwrap();
        std::fs::write(second.path().join("one"), b"1").unwrap();
        std::fs::write(second.path().join("two"), b"2").unwrap();

        let candidates = vec![
            ResourceRootCandidate {
                source: "first",
                path: first.path().to_path_buf(),
            },
            ResourceRootCandidate {
                source: "second",
                path: second.path().to_path_buf(),
            },
        ];

        let selected = select_resource_root(&candidates, &["one", "two"]).unwrap();
        assert_eq!(selected.source, "first");
    }
}
