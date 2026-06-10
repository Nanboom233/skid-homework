#![cfg(any(target_os = "windows", target_os = "linux"))]

mod scanner_resource_under_test {
    #![allow(dead_code)]

    include!("../src/scanner_resource.rs");

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
}
