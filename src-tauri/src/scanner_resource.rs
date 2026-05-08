/// Shared resource directory resolution utilities.
///
/// Both the detection and post-process model subsystems need to locate
/// the `resources/` directory that ships alongside the Tauri bundle.
/// This module provides a single implementation so that the resolution
/// logic (candidate enumeration, scoring, selection) stays DRY.
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A single candidate resource root with provenance metadata.
#[derive(Debug, Clone)]
pub struct ResourceRootCandidate {
    pub source: &'static str,
    pub path: PathBuf,
}

/// Build an ordered list of candidate resource directories.
///
/// The list always includes the Cargo manifest `resources/` directory
/// and any CWD-relative fallbacks.  When a Tauri `resource_dir` hint
/// is available it is inserted first.
pub fn build_resource_root_candidates(
    resource_dir_hint: Option<PathBuf>,
) -> Vec<ResourceRootCandidate> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    let mut push_candidate = |source: &'static str, path: PathBuf| {
        if seen.insert(path.clone()) {
            candidates.push(ResourceRootCandidate { source, path });
        }
    };

    if let Some(resource_dir) = resource_dir_hint {
        push_candidate("tauri-resource-dir", resource_dir);
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    push_candidate("cargo-manifest-resources", manifest_dir.join("resources"));

    // CWD-based fallbacks are only safe during development; in release
    // builds the CWD is user-controlled and could point at a hostile tree.
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

/// Select the best candidate by scoring each root against a set of
/// expected relative paths.
pub fn select_resource_root(
    candidates: &[ResourceRootCandidate],
    interesting_paths: &[&str],
) -> Option<ResourceRootCandidate> {
    candidates
        .iter()
        .filter(|c| score_resource_root(&c.path, interesting_paths) > 0)
        .max_by_key(|candidate| score_resource_root(&candidate.path, interesting_paths))
        .cloned()
}

/// Score a resource root by counting how many of the `interesting_paths`
/// actually exist on disk.
pub fn score_resource_root(root: &Path, interesting_paths: &[&str]) -> usize {
    interesting_paths
        .iter()
        .filter(|relative_path| root.join(relative_path).exists())
        .count()
}

/// Convert a path to a display-friendly string (lossy).
pub fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Validate that a resolved path is contained within the expected root.
/// Returns an error if the path escapes the root via `..` traversal.
pub fn validate_path_containment(path: &Path, root: &Path) -> Result<PathBuf, String> {
    let canonical = path.canonicalize().map_err(|e| {
        format!("Failed to canonicalize path {:?}: {}", path, e)
    })?;
    let canonical_root = root.canonicalize().map_err(|e| {
        format!("Failed to canonicalize root {:?}: {}", root, e)
    })?;
    if !canonical.starts_with(&canonical_root) {
        return Err(format!(
            "Path traversal detected: {:?} is not under {:?}",
            canonical, canonical_root,
        ));
    }
    Ok(canonical)
}
