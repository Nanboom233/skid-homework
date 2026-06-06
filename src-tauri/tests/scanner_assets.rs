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
                "assetVersion": EXPECTED_SCANNER_ASSET_TAG,
                "platformTarget": scanner_platform::platform_target(),
                "files": files,
            });
            fs::write(
                root.join(MANIFEST_FILE_NAME),
                serde_json::to_vec_pretty(&manifest).unwrap(),
            )
            .unwrap();
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

            let error = verify_manifest_metadata(&manifest).expect_err("schema version must fail");
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

            let error =
                verify_manifest_metadata(&manifest).expect_err("platform mismatch must fail");
            assert_eq!(error.code, "assets.install.manifest.platformMismatch");
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
        fn operation_mutex_rejects_concurrent_operations() {
            let guard = try_acquire_operation(OperationKind::Importing).unwrap();
            let error = try_acquire_operation(OperationKind::Downloading)
                .expect_err("second operation must fail");
            assert_eq!(error.code, "assets.operation.inProgress");
            drop(guard);

            let guard = try_acquire_operation(OperationKind::Downloading).unwrap();
            drop(guard);
        }

        #[test]
        fn cleanup_staging_preserves_current_directory() {
            let temp_dir = tempfile::tempdir().unwrap();
            let paths = asset_paths_from_app_data_dir(temp_dir.path().to_path_buf());
            fs::create_dir_all(&paths.current_dir).unwrap();
            fs::write(paths.current_dir.join("marker"), b"current").unwrap();
            fs::create_dir_all(&paths.staging_dir).unwrap();
            fs::write(paths.staging_dir.join("marker"), b"staging").unwrap();

            fs::remove_dir_all(&paths.staging_dir).unwrap();

            assert_eq!(
                fs::read(paths.current_dir.join("marker")).unwrap(),
                b"current"
            );
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
