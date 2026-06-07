/// Shared resource directory resolution utilities.
///
/// Installed scanner assets are preferred when available, but the original
/// scanner workflow also supported development resources under `src-tauri/resources`.
/// Keep both paths so local scanner/postprocess behavior matches the baseline
/// while still allowing runtime-downloaded assets to take precedence.
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

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    push_candidate("cargo-manifest-resources", manifest_dir.join("resources"));

    #[cfg(debug_assertions)]
    if let Ok(current_dir) = std::env::current_dir() {
        push_candidate("cwd-resources", current_dir.join("resources"));
        push_candidate(
            "cwd-src-tauri-resources",
            current_dir.join("src-tauri").join("resources"),
        );
    }

    candidates
}

pub fn select_resource_root(
    candidates: &[ResourceRootCandidate],
    interesting_paths: &[&str],
) -> Option<ResourceRootCandidate> {
    candidates
        .iter()
        .filter(|candidate| score_resource_root(&candidate.path, interesting_paths) > 0)
        .max_by_key(|candidate| score_resource_root(&candidate.path, interesting_paths))
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
