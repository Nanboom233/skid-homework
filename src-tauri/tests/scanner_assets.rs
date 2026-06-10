#![cfg(any(target_os = "windows", target_os = "linux"))]

#[path = "../src/scanner_platform.rs"]
#[allow(dead_code)]
mod scanner_platform;
#[path = "../src/scanner_resource.rs"]
#[allow(dead_code)]
mod scanner_resource;

mod scanner_assets_under_test {
    #![allow(dead_code)]

    include!("../src/scanner_assets.rs");

    #[cfg(test)]
    mod tests {
        use super::*;

        fn parse_manifest_bytes(bytes: &[u8]) -> Result<ScannerAssetManifest, ScannerAssetsError> {
            serde_json::from_slice(bytes).map_err(|error| {
                ScannerAssetsError::with_details(
                    "assets.install.manifest.invalid",
                    false,
                    format!("Failed to parse scanner asset manifest: {error}"),
                )
            })
        }

        fn manifest_with_files(files: Vec<ScannerAssetManifestFile>) -> ScannerAssetManifest {
            ScannerAssetManifest {
                schema_version: 1,
                asset_version: EXPECTED_SCANNER_ASSET_TAG.to_string(),
                platform_target: scanner_platform::platform_target().to_string(),
                files,
            }
        }

        fn manifest_file(path: &str, bytes: &[u8]) -> ScannerAssetManifestFile {
            ScannerAssetManifestFile {
                path: path.to_string(),
                size: bytes.len() as u64,
                sha256: hex_lower(&Sha256::digest(bytes)),
                source_url: format!("https://official.example/{path}"),
            }
        }

        fn write_complete_install_root(root: &Path) {
            write_complete_install_root_with_version(root, EXPECTED_SCANNER_ASSET_TAG);
        }

        fn write_complete_install_root_with_version(root: &Path, asset_version: &str) {
            let mut files = Vec::new();

            for relative_path in required_asset_paths_for_current_platform() {
                let bytes = format!("test asset for {relative_path}").into_bytes();
                let file_path = root.join(validate_manifest_relative_path(relative_path).unwrap());
                fs::create_dir_all(file_path.parent().unwrap()).unwrap();
                fs::write(&file_path, &bytes).unwrap();
                files.push(serde_json::json!({
                    "path": relative_path,
                    "size": bytes.len() as u64,
                    "sha256": hex_lower(&Sha256::digest(&bytes)),
                    "sourceUrl": format!("https://official.example/{relative_path}"),
                }));
            }

            let manifest = serde_json::json!({
                "schemaVersion": 1,
                "assetVersion": asset_version,
                "platformTarget": scanner_platform::platform_target(),
                "files": files,
            });
            fs::write(
                root.join(MANIFEST_FILE_NAME),
                serde_json::to_vec_pretty(&manifest).unwrap(),
            )
            .unwrap();
        }

        fn github_release(
            tag_name: &str,
            draft: bool,
            prerelease: bool,
            asset_names: &[String],
        ) -> GitHubRelease {
            GitHubRelease {
                tag_name: tag_name.to_string(),
                draft,
                prerelease,
                assets: asset_names
                    .iter()
                    .map(|name| GitHubReleaseAsset { name: name.clone() })
                    .collect(),
            }
        }

        fn asset_with_checksum(file_name: String) -> Vec<String> {
            vec![file_name.clone(), format!("{file_name}.sha256")]
        }

        #[test]
        fn package_file_name_and_default_url_use_expected_tag_without_prefix() {
            let file_name = platform_package_file_name();
            let expected_release_prefix = format!(
                "https://github.com/{SCANNER_ASSETS_REPO_OWNER}/{SCANNER_ASSETS_REPO_NAME}/releases/download/{EXPECTED_SCANNER_ASSET_TAG}/"
            );
            assert!(file_name.contains(EXPECTED_SCANNER_ASSET_TAG));
            assert!(!file_name.starts_with("scanner-assets-"));
            assert!(default_asset_url().ends_with(&file_name));
            assert!(default_asset_url().starts_with(&expected_release_prefix));
            assert!(!default_asset_url().contains("/latest/"));
        }

        #[test]
        fn release_target_selection_uses_first_suitable_platform_asset() {
            let v010 = platform_package_file_name_for_tag("v0.1.0");
            let v020 = platform_package_file_name_for_tag("v0.2.0");
            let v100 = platform_package_file_name_for_tag("v1.0.0");
            let releases = vec![
                github_release(
                    "v0.2.0-rc.1",
                    false,
                    true,
                    &asset_with_checksum(platform_package_file_name_for_tag("v0.2.0-rc.1")),
                ),
                github_release("v1.0.0", true, false, &[v100]),
                github_release(
                    "v0.10.0",
                    false,
                    false,
                    &[platform_package_file_name_for_tag("other")],
                ),
                github_release("v0.2.0", false, false, &asset_with_checksum(v020.clone())),
                github_release("v0.1.0", false, false, &asset_with_checksum(v010)),
            ];

            let target = select_latest_release_target(&releases).unwrap();

            assert_eq!(target.asset_tag, "v0.2.0");
            assert_eq!(target.package_file_name, v020);
            assert_eq!(
                target.asset_url,
                official_release_asset_url("v0.2.0", &target.package_file_name)
            );
        }

        #[test]
        fn release_target_selection_skips_newer_releases_without_ort_assets() {
            let other_asset = "camera-server-v0.3.0.jar".to_string();
            let ort_asset = platform_package_file_name_for_tag("v0.2.0");
            let releases = vec![
                github_release("v0.3.0", false, false, &asset_with_checksum(other_asset)),
                github_release(
                    "v0.2.0",
                    false,
                    false,
                    &asset_with_checksum(ort_asset.clone()),
                ),
            ];

            let target = select_latest_release_target(&releases).unwrap();

            assert_eq!(target.asset_tag, "v0.2.0");
            assert_eq!(target.package_file_name, ort_asset);
        }

        #[test]
        fn release_target_selection_allows_prerelease_release() {
            let prerelease_asset = platform_package_file_name_for_tag("v0.3.0");
            let stable_asset = platform_package_file_name_for_tag("v0.2.0");
            let releases = vec![
                github_release(
                    "v0.3.0",
                    false,
                    true,
                    &asset_with_checksum(prerelease_asset.clone()),
                ),
                github_release("v0.2.0", false, false, &asset_with_checksum(stable_asset)),
            ];

            let target = select_latest_release_target(&releases).unwrap();

            assert_eq!(target.asset_tag, "v0.3.0");
            assert_eq!(target.package_file_name, prerelease_asset);
        }

        #[test]
        fn release_target_selection_requires_archive_checksum_sidecar() {
            let file_name = platform_package_file_name_for_tag("v0.2.0");
            let releases = vec![github_release("v0.2.0", false, false, &[file_name])];

            assert!(select_latest_release_target(&releases).is_none());
        }

        #[test]
        fn release_target_selection_uses_newest_first_release_order() {
            let releases = vec![
                github_release(
                    "v0.9.0",
                    false,
                    false,
                    &asset_with_checksum(platform_package_file_name_for_tag("v0.9.0")),
                ),
                github_release(
                    "v0.10.0",
                    false,
                    false,
                    &asset_with_checksum(platform_package_file_name_for_tag("v0.10.0")),
                ),
            ];

            let target = select_latest_release_target(&releases).unwrap();

            assert_eq!(target.asset_tag, "v0.9.0");
        }

        #[test]
        fn update_available_compares_against_official_target_without_downgrading() {
            assert!(is_update_available(None, "v0.2.0"));
            assert!(is_update_available(Some("v0.1.0"), "v0.2.0"));
            assert!(!is_update_available(Some("v0.2.0"), "v0.2.0"));
            assert!(!is_update_available(Some("v9.9.9"), "v0.2.0"));
            assert!(is_update_available(Some("custom-build"), "v0.2.0"));
        }

        #[test]
        fn download_timeouts_are_configured() {
            assert_eq!(DOWNLOAD_CONNECT_TIMEOUT, Duration::from_secs(15));
            assert_eq!(DOWNLOAD_READ_TIMEOUT, Duration::from_secs(60));
        }

        #[test]
        fn manifest_rejects_unknown_fields() {
            let manifest = br#"{
                "schemaVersion": 1,
                "assetVersion": "v0.1.0",
                "platformTarget": "windows-directml",
                "archiveFormat": "zip",
                "files": []
            }"#;

            let error = parse_manifest_bytes(manifest).expect_err("unknown fields must fail");
            assert_eq!(error.code, "assets.install.manifest.invalid");
        }

        #[test]
        fn manifest_rejects_unsupported_schema_version() {
            let manifest = ScannerAssetManifest {
                schema_version: 2,
                asset_version: EXPECTED_SCANNER_ASSET_TAG.to_string(),
                platform_target: scanner_platform::platform_target().to_string(),
                files: vec![manifest_file("models/uvdoc-best-model.onnx", b"model")],
            };

            let error = verify_manifest_metadata(&manifest, ManifestVersionPolicy::AllowAny)
                .expect_err("schema version must fail");
            assert_eq!(error.code, "assets.install.manifest.invalid");
        }

        #[test]
        fn unsafe_manifest_paths_are_rejected() {
            for path in [
                "",
                "../model.onnx",
                "models/../model.onnx",
                "/models/model.onnx",
                "//server/share/model.onnx",
                r"models\model.onnx",
                "C:/models/model.onnx",
                "./models/model.onnx",
            ] {
                let error = validate_manifest_relative_path(path).expect_err(path);
                assert_eq!(error.code, "assets.install.verify.unsafePath");
            }

            assert_eq!(
                validate_manifest_relative_path("models/uvdoc-best-model.onnx").unwrap(),
                PathBuf::from("models").join("uvdoc-best-model.onnx")
            );
        }

        #[test]
        fn platform_mismatch_is_rejected() {
            let manifest = ScannerAssetManifest {
                schema_version: 1,
                asset_version: EXPECTED_SCANNER_ASSET_TAG.to_string(),
                platform_target: "wrong-platform".to_string(),
                files: vec![manifest_file("models/uvdoc-best-model.onnx", b"model")],
            };

            let error = verify_manifest_metadata(&manifest, ManifestVersionPolicy::AllowAny)
                .expect_err("platform mismatch must fail");
            assert_eq!(error.code, "assets.install.manifest.platformMismatch");
        }

        #[test]
        fn strict_manifest_metadata_rejects_version_mismatch() {
            let manifest = ScannerAssetManifest {
                schema_version: 1,
                asset_version: "v9.9.9".to_string(),
                platform_target: scanner_platform::platform_target().to_string(),
                files: vec![manifest_file("models/uvdoc-best-model.onnx", b"model")],
            };

            let error = verify_manifest_metadata(
                &manifest,
                ManifestVersionPolicy::RequireTag(EXPECTED_SCANNER_ASSET_TAG.to_string()),
            )
            .expect_err("strict download validation must reject mismatched versions");

            assert_eq!(error.code, "assets.install.manifest.versionMismatch");
        }

        #[test]
        fn relaxed_manifest_metadata_accepts_version_mismatch() {
            let manifest = ScannerAssetManifest {
                schema_version: 1,
                asset_version: "v9.9.9".to_string(),
                platform_target: scanner_platform::platform_target().to_string(),
                files: vec![manifest_file("models/uvdoc-best-model.onnx", b"model")],
            };

            verify_manifest_metadata(&manifest, ManifestVersionPolicy::AllowAny)
                .expect("local import and status validation should allow a different assetVersion");
        }

        #[test]
        fn missing_required_declaration_is_rejected() {
            let manifest = manifest_with_files(vec![manifest_file(
                "models/uvdoc-best-model.onnx",
                b"model",
            )]);

            let error = verify_required_file_declarations(&manifest)
                .expect_err("missing required must fail");
            assert_eq!(error.code, "assets.install.verify.fileMissing");
        }

        #[test]
        fn declared_missing_file_is_rejected() {
            let temp_dir = tempfile::tempdir().unwrap();
            let manifest =
                manifest_with_files(vec![manifest_file("models/missing.onnx", b"model")]);

            let error = verify_declared_files(temp_dir.path(), &manifest)
                .expect_err("missing file must fail");
            assert_eq!(error.code, "assets.install.verify.fileMissing");
        }

        #[test]
        fn declared_size_mismatch_is_rejected() {
            let temp_dir = tempfile::tempdir().unwrap();
            let file_path = temp_dir.path().join("models").join("model.onnx");
            fs::create_dir_all(file_path.parent().unwrap()).unwrap();
            fs::write(&file_path, b"actual").unwrap();
            let mut entry = manifest_file("models/model.onnx", b"actual");
            entry.size += 1;
            let manifest = manifest_with_files(vec![entry]);

            let error = verify_declared_files(temp_dir.path(), &manifest)
                .expect_err("size mismatch must fail");
            assert_eq!(error.code, "assets.install.verify.sizeMismatch");
        }

        #[test]
        fn declared_checksum_mismatch_is_rejected() {
            let temp_dir = tempfile::tempdir().unwrap();
            let file_path = temp_dir.path().join("models").join("model.onnx");
            fs::create_dir_all(file_path.parent().unwrap()).unwrap();
            fs::write(&file_path, b"actual").unwrap();
            let entry = manifest_file("models/model.onnx", b"other!");
            let manifest = manifest_with_files(vec![entry]);

            let error = verify_declared_files(temp_dir.path(), &manifest)
                .expect_err("checksum mismatch must fail");
            assert_eq!(error.code, "assets.install.verify.checksumMismatch");
        }

        #[test]
        fn operation_mutex_rejects_concurrent_operations_and_cancel_targets_download() {
            let guard = try_acquire_operation(OperationKind::Importing).unwrap();
            let error = try_acquire_operation(OperationKind::Clearing)
                .expect_err("second operation must fail");
            assert_eq!(error.code, "assets.operation.inProgress");
            drop(guard);

            let (guard, cancel_requested) =
                try_acquire_download_operation("download-op".to_string()).unwrap();
            request_operation_cancel("stale-op");
            assert!(!cancel_requested.load(Ordering::SeqCst));
            request_operation_cancel("download-op");
            assert!(cancel_requested.load(Ordering::SeqCst));
            drop(guard);
        }

        #[test]
        fn clear_installed_assets_stops_worker_before_removing_current_paths() {
            TEST_ORT_ASSET_MUTATION_CALLS.with(|calls| calls.set(0));
            TEST_ORT_ASSET_MUTATION_FAIL.with(|fail| fail.set(false));
            let temp_dir = tempfile::tempdir().unwrap();
            let paths = asset_paths_from_app_data_dir(temp_dir.path().to_path_buf());
            fs::create_dir_all(paths.current_dir.join("models")).unwrap();
            fs::create_dir_all(paths.current_dir.join("onnxruntime")).unwrap();
            fs::write(paths.current_dir.join(MANIFEST_FILE_NAME), b"manifest").unwrap();
            fs::write(paths.current_dir.join("models/model.onnx"), b"model").unwrap();
            fs::write(
                paths.current_dir.join("onnxruntime/runtime.dll"),
                b"runtime",
            )
            .unwrap();

            clear_installed_assets(&paths).unwrap();

            TEST_ORT_ASSET_MUTATION_CALLS.with(|calls| assert_eq!(calls.get(), 1));
            assert!(!paths.current_dir.join(MANIFEST_FILE_NAME).exists());
            assert!(!paths.current_dir.join("models").exists());
            assert!(!paths.current_dir.join("onnxruntime").exists());
        }

        #[test]
        fn clear_installed_assets_preserves_current_when_worker_stop_fails() {
            TEST_ORT_ASSET_MUTATION_CALLS.with(|calls| calls.set(0));
            TEST_ORT_ASSET_MUTATION_FAIL.with(|fail| fail.set(true));
            let temp_dir = tempfile::tempdir().unwrap();
            let paths = asset_paths_from_app_data_dir(temp_dir.path().to_path_buf());
            fs::create_dir_all(paths.current_dir.join("models")).unwrap();
            fs::create_dir_all(paths.current_dir.join("onnxruntime")).unwrap();
            fs::write(paths.current_dir.join(MANIFEST_FILE_NAME), b"manifest").unwrap();
            fs::write(paths.current_dir.join("models/model.onnx"), b"model").unwrap();
            fs::write(
                paths.current_dir.join("onnxruntime/runtime.dll"),
                b"runtime",
            )
            .unwrap();

            let error =
                clear_installed_assets(&paths).expect_err("worker stop failure must abort clear");

            TEST_ORT_ASSET_MUTATION_FAIL.with(|fail| fail.set(false));
            assert_eq!(error.code, "runtime.worker.stopFailed");
            TEST_ORT_ASSET_MUTATION_CALLS.with(|calls| assert_eq!(calls.get(), 1));
            assert!(paths.current_dir.join(MANIFEST_FILE_NAME).exists());
            assert!(paths.current_dir.join("models/model.onnx").exists());
            assert!(paths.current_dir.join("onnxruntime/runtime.dll").exists());
        }

        #[test]
        fn cancellation_error_is_not_retryable() {
            let cancel_requested = AtomicBool::new(true);
            let error = check_cancelled(&cancel_requested).expect_err("cancelled must fail");

            assert_eq!(error.code, ERROR_OPERATION_CANCELLED);
            assert!(!error.retryable);
        }

        #[test]
        fn failed_download_cleanup_preserves_current_directory() {
            let temp_dir = tempfile::tempdir().unwrap();
            let paths = asset_paths_from_app_data_dir(temp_dir.path().to_path_buf());
            fs::create_dir_all(&paths.current_dir).unwrap();
            fs::write(paths.current_dir.join("marker"), b"current").unwrap();
            fs::create_dir_all(&paths.staging_dir).unwrap();
            fs::write(paths.staging_dir.join("marker"), b"staging").unwrap();
            let archive_path = paths.assets_dir.join(".download-test");
            fs::write(&archive_path, b"download").unwrap();

            cleanup_failed_download(&paths, &archive_path, true);

            assert_eq!(
                fs::read(paths.current_dir.join("marker")).unwrap(),
                b"current"
            );
            assert!(!paths.staging_dir.exists());
            assert!(!archive_path.exists());
        }

        #[test]
        fn clear_installed_assets_removes_current_and_transient_residue() {
            let temp_dir = tempfile::tempdir().unwrap();
            let paths = asset_paths_from_app_data_dir(temp_dir.path().to_path_buf());
            fs::create_dir_all(&paths.current_dir).unwrap();
            fs::write(paths.current_dir.join("marker"), b"current").unwrap();
            fs::create_dir_all(&paths.staging_dir).unwrap();
            fs::write(paths.staging_dir.join("marker"), b"staging").unwrap();
            fs::create_dir_all(&paths.backup_dir).unwrap();
            fs::write(paths.backup_dir.join("marker"), b"backup").unwrap();
            let numbered_backup = paths.assets_dir.join("current.previous.1");
            fs::create_dir_all(&numbered_backup).unwrap();
            fs::write(numbered_backup.join("marker"), b"backup").unwrap();
            let archive_path = paths.assets_dir.join(".download-test");
            fs::write(&archive_path, b"download").unwrap();
            set_last_error(ScannerAssetsError::with_details(
                "assets.install.verify.fileMissing",
                true,
                "stale error",
            ));

            clear_installed_assets(&paths).unwrap();

            assert!(paths.assets_dir.exists());
            assert!(!paths.current_dir.exists());
            assert!(!paths.staging_dir.exists());
            assert!(!paths.backup_dir.exists());
            assert!(!numbered_backup.exists());
            assert!(!archive_path.exists());
            assert!(last_error().is_none());
        }

        #[test]
        fn strict_install_root_rejects_different_asset_version() {
            let temp_dir = tempfile::tempdir().unwrap();
            write_complete_install_root_with_version(temp_dir.path(), "v9.9.9");

            let error = load_and_verify_install_root(
                temp_dir.path(),
                ManifestVersionPolicy::RequireTag(EXPECTED_SCANNER_ASSET_TAG.to_string()),
            )
            .expect_err("download install validation must reject mismatched versions");

            assert_eq!(error.code, "assets.install.manifest.versionMismatch");
        }

        #[test]
        fn relaxed_install_root_accepts_different_asset_version() {
            let temp_dir = tempfile::tempdir().unwrap();
            write_complete_install_root_with_version(temp_dir.path(), "v9.9.9");

            let manifest =
                load_and_verify_install_root(temp_dir.path(), ManifestVersionPolicy::AllowAny)
                    .expect("local import validation should accept a different assetVersion");

            assert_eq!(manifest.asset_version, "v9.9.9");
        }

        #[test]
        fn inspect_current_install_reports_different_asset_version_as_ready() {
            let temp_dir = tempfile::tempdir().unwrap();
            let paths = asset_paths_from_app_data_dir(temp_dir.path().to_path_buf());
            fs::create_dir_all(&paths.current_dir).unwrap();
            write_complete_install_root_with_version(&paths.current_dir, "v9.9.9");

            let summary = inspect_current_install(&paths.current_dir).unwrap();

            assert_eq!(summary.schema_version, 1);
            assert_eq!(summary.asset_version, "v9.9.9");
            assert_eq!(summary.platform_target, scanner_platform::platform_target());
        }

        #[test]
        fn activate_staging_uses_unique_backup_when_stale_backup_remains() {
            let temp_dir = tempfile::tempdir().unwrap();
            let paths = asset_paths_from_app_data_dir(temp_dir.path().to_path_buf());
            fs::create_dir_all(&paths.current_dir).unwrap();
            fs::write(paths.current_dir.join("marker"), b"current").unwrap();
            fs::create_dir_all(&paths.staging_dir).unwrap();
            fs::write(paths.staging_dir.join("marker"), b"staging").unwrap();
            fs::write(&paths.backup_dir, b"locked-backup-placeholder").unwrap();

            activate_staging(&paths).unwrap();

            assert_eq!(
                fs::read(paths.current_dir.join("marker")).unwrap(),
                b"staging"
            );
            assert_eq!(
                fs::read(&paths.backup_dir).unwrap(),
                b"locked-backup-placeholder"
            );
            assert!(!paths.assets_dir.join("current.previous.1").exists());
        }

        #[test]
        fn activate_staging_exposes_new_current_manifest_when_stale_backup_remains() {
            let temp_dir = tempfile::tempdir().unwrap();
            let paths = asset_paths_from_app_data_dir(temp_dir.path().to_path_buf());
            fs::create_dir_all(&paths.current_dir).unwrap();
            fs::write(paths.current_dir.join("marker"), b"current").unwrap();
            fs::create_dir_all(&paths.staging_dir).unwrap();
            write_complete_install_root(&paths.staging_dir);
            fs::write(&paths.backup_dir, b"locked-backup-placeholder").unwrap();

            activate_staging(&paths).unwrap();

            let summary = inspect_current_install(&paths.current_dir).unwrap();
            assert_eq!(summary.schema_version, 1);
            assert_eq!(summary.asset_version, EXPECTED_SCANNER_ASSET_TAG);
            assert_eq!(summary.platform_target, scanner_platform::platform_target());
            assert_eq!(
                fs::read(&paths.backup_dir).unwrap(),
                b"locked-backup-placeholder"
            );
            assert!(!paths.assets_dir.join("current.previous.1").exists());
        }

        #[test]
        fn activate_staging_rolls_back_current_when_activation_fails() {
            let temp_dir = tempfile::tempdir().unwrap();
            let paths = asset_paths_from_app_data_dir(temp_dir.path().to_path_buf());
            fs::create_dir_all(&paths.current_dir).unwrap();
            fs::write(paths.current_dir.join("marker"), b"current").unwrap();
            fs::write(&paths.backup_dir, b"locked-backup-placeholder").unwrap();

            let error = activate_staging(&paths).expect_err("missing staging must fail");

            assert_eq!(error.code, "assets.install.activate.replaceFailed");
            assert_eq!(
                fs::read(paths.current_dir.join("marker")).unwrap(),
                b"current"
            );
            assert_eq!(
                fs::read(&paths.backup_dir).unwrap(),
                b"locked-backup-placeholder"
            );
            assert!(!paths.assets_dir.join("current.previous.1").exists());
        }
    }
}
