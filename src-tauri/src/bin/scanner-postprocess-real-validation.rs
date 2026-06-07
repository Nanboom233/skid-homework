use std::fs;
use std::path::{Path, PathBuf};

use app_lib::scanner_detect::{
    detect_document_native_ort, ScannerDetectDocumentRequest, ScannerDetectDocumentResponse,
    ScannerPoint,
};
use app_lib::scanner_postprocess::{
    postprocess_image_bytes_with_options, ScannerPostProcessResponse,
};
use serde::{Deserialize, Serialize};

#[derive(Debug)]
struct Args {
    image_path: PathBuf,
    output_dir: PathBuf,
    points_json: Option<PathBuf>,
    output_rotation: u16,
    image_enhancement: bool,
    postprocess_backend: String,
    resource_root: Option<PathBuf>,
    skip_detect: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SidecarPointsObject {
    document_points: Option<Vec<ScannerPoint>>,
    points: Option<Vec<ScannerPoint>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SidecarPointsFile {
    Points(Vec<ScannerPoint>),
    Object(SidecarPointsObject),
}

impl SidecarPointsFile {
    fn into_points(self) -> Vec<ScannerPoint> {
        match self {
            Self::Points(points) => points,
            Self::Object(object) => object.document_points.or(object.points).unwrap_or_default(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationMetadata {
    image_path: String,
    input_copy_path: String,
    output_image_path: Option<String>,
    points_source: &'static str,
    points_sidecar_path: Option<String>,
    resolved_points: Option<Vec<ScannerPoint>>,
    output_rotation: u16,
    image_enhancement: bool,
    postprocess_backend: String,
    resource_root: Option<String>,
    detection: Option<ScannerDetectDocumentResponse>,
    postprocess: Option<ScannerPostProcessResponse>,
    status: &'static str,
    error: Option<String>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = parse_args(std::env::args().skip(1))?;
    let image_bytes = fs::read(&args.image_path)
        .map_err(|error| format!("Failed to read {}: {error}", args.image_path.display()))?;

    fs::create_dir_all(&args.output_dir).map_err(|error| {
        format!(
            "Failed to create output directory {}: {error}",
            args.output_dir.display()
        )
    })?;

    let input_copy_path = build_input_copy_path(&args.output_dir, &args.image_path);
    fs::write(&input_copy_path, &image_bytes).map_err(|error| {
        format!(
            "Failed to write input copy {}: {error}",
            input_copy_path.display()
        )
    })?;

    let metadata_path = args.output_dir.join("validation.json");
    let output_image_path = args.output_dir.join("output.png");

    let (points_source, points_sidecar_path, resolved_points, detection_response) =
        resolve_document_points(&args, &image_bytes)?;

    let Some(points) = resolved_points.as_deref() else {
        let metadata = ValidationMetadata {
            image_path: args.image_path.display().to_string(),
            input_copy_path: input_copy_path.display().to_string(),
            output_image_path: None,
            points_source,
            points_sidecar_path: points_sidecar_path
                .as_ref()
                .map(|path| path.display().to_string()),
            resolved_points,
            output_rotation: args.output_rotation,
            image_enhancement: args.image_enhancement,
            postprocess_backend: args.postprocess_backend.clone(),
            resource_root: args
                .resource_root
                .as_ref()
                .map(|path| path.display().to_string()),
            detection: detection_response,
            postprocess: None,
            status: "failed",
            error: Some(
                "No document points were available from sidecar input or Native ORT detection."
                    .to_string(),
            ),
        };
        write_validation_metadata(&metadata_path, &metadata)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&metadata).expect("validation metadata should serialize")
        );
        return Err(format!(
            "Validation failed for {} because no document points were available.",
            args.image_path.display()
        ));
    };

    let (postprocess_response, encoded_png) = match postprocess_image_bytes_with_options(
        image_bytes,
        Some(points),
        args.output_rotation,
        args.image_enhancement,
        "auto".to_string(),
        args.postprocess_backend.clone(),
        args.resource_root.clone(),
        None,
        true,
        true,
        "none".to_string(),
    ) {
        Ok(result) => result,
        Err(error) => {
            let metadata = ValidationMetadata {
                image_path: args.image_path.display().to_string(),
                input_copy_path: input_copy_path.display().to_string(),
                output_image_path: None,
                points_source,
                points_sidecar_path: points_sidecar_path
                    .as_ref()
                    .map(|path| path.display().to_string()),
                resolved_points,
                output_rotation: args.output_rotation,
                image_enhancement: args.image_enhancement,
                postprocess_backend: args.postprocess_backend.clone(),
                resource_root: args
                    .resource_root
                    .as_ref()
                    .map(|path| path.display().to_string()),
                detection: detection_response,
                postprocess: None,
                status: "failed",
                error: Some(error),
            };
            write_validation_metadata(&metadata_path, &metadata)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&metadata)
                    .expect("validation metadata should serialize")
            );
            return Err(format!(
                "Validation failed while postprocessing {}.",
                args.image_path.display()
            ));
        }
    };

    fs::write(&output_image_path, &encoded_png).map_err(|error| {
        format!(
            "Failed to write processed output {}: {error}",
            output_image_path.display()
        )
    })?;

    let metadata = ValidationMetadata {
        image_path: args.image_path.display().to_string(),
        input_copy_path: input_copy_path.display().to_string(),
        output_image_path: Some(output_image_path.display().to_string()),
        points_source,
        points_sidecar_path: points_sidecar_path
            .as_ref()
            .map(|path| path.display().to_string()),
        resolved_points,
        output_rotation: args.output_rotation,
        image_enhancement: args.image_enhancement,
        postprocess_backend: args.postprocess_backend.clone(),
        resource_root: args
            .resource_root
            .as_ref()
            .map(|path| path.display().to_string()),
        detection: detection_response,
        postprocess: Some(postprocess_response),
        status: "ok",
        error: None,
    };
    write_validation_metadata(&metadata_path, &metadata)?;

    println!(
        "{}",
        serde_json::to_string_pretty(&metadata).expect("validation metadata should serialize")
    );
    Ok(())
}

fn parse_args<I>(args: I) -> Result<Args, String>
where
    I: IntoIterator<Item = String>,
{
    let mut image_path = None;
    let mut output_dir = None;
    let mut points_json = None;
    let mut output_rotation = 0u16;
    let mut image_enhancement = true;
    let mut postprocess_backend = "heuristic".to_string();
    let mut resource_root = None;
    let mut skip_detect = false;

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--output-dir" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "Missing value for --output-dir.".to_string())?;
                output_dir = Some(PathBuf::from(value));
            }
            "--points-json" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "Missing value for --points-json.".to_string())?;
                points_json = Some(PathBuf::from(value));
            }
            "--rotation" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "Missing value for --rotation.".to_string())?;
                output_rotation = parse_rotation(&value)?;
            }
            "--no-enhance" => {
                image_enhancement = false;
            }
            "--postprocess-backend" => {
                postprocess_backend = iter
                    .next()
                    .ok_or_else(|| "Missing value for --postprocess-backend.".to_string())?;
            }
            "--resource-root" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "Missing value for --resource-root.".to_string())?;
                resource_root = Some(PathBuf::from(value));
            }
            "--skip-detect" => {
                skip_detect = true;
            }
            "--help" | "-h" => {
                return Err(usage().to_string());
            }
            other if other.starts_with("--") => {
                return Err(format!("Unknown argument: {other}\n\n{}", usage()));
            }
            other => {
                if image_path.is_some() {
                    return Err(format!(
                        "Unexpected extra positional argument {other:?}.\n\n{}",
                        usage()
                    ));
                }
                image_path = Some(PathBuf::from(other));
            }
        }
    }

    Ok(Args {
        image_path: image_path.ok_or_else(|| usage().to_string())?,
        output_dir: output_dir.ok_or_else(|| usage().to_string())?,
        points_json,
        output_rotation,
        image_enhancement,
        postprocess_backend,
        resource_root,
        skip_detect,
    })
}

fn usage() -> &'static str {
    "usage: scanner-postprocess-real-validation <image-path> --output-dir <dir> [--points-json <path>] [--rotation <0|90|180|270>] [--no-enhance] [--postprocess-backend <heuristic|native-ml-v1>] [--resource-root <dir>] [--skip-detect]"
}

fn parse_rotation(value: &str) -> Result<u16, String> {
    let rotation = value
        .parse::<u16>()
        .map_err(|error| format!("Invalid --rotation value {value:?}: {error}"))?;
    match rotation {
        0 | 90 | 180 | 270 => Ok(rotation),
        _ => Err(format!(
            "Unsupported --rotation value {rotation}. Expected 0, 90, 180, or 270."
        )),
    }
}

fn build_input_copy_path(output_dir: &Path, image_path: &Path) -> PathBuf {
    let extension = image_path
        .extension()
        .map(|value| format!(".{}", value.to_string_lossy()))
        .unwrap_or_else(|| ".bin".to_string());
    output_dir.join(format!("input{extension}"))
}

fn resolve_document_points(
    args: &Args,
    image_bytes: &[u8],
) -> Result<
    (
        &'static str,
        Option<PathBuf>,
        Option<Vec<ScannerPoint>>,
        Option<ScannerDetectDocumentResponse>,
    ),
    String,
> {
    if let Some(sidecar_path) =
        resolve_points_sidecar_path(&args.image_path, args.points_json.as_deref())
    {
        let points = read_sidecar_points(&sidecar_path)?;
        return Ok(("sidecar", Some(sidecar_path), Some(points), None));
    }

    if args.skip_detect {
        return Ok(("none", None, None, None));
    }

    let detection_response = detect_document_native_ort(
        ScannerDetectDocumentRequest {
            source_bytes: image_bytes.to_vec(),
            rgba_bytes: Vec::new(),
            use_latest_preview_frame: false,
            rgba_width: None,
            rgba_height: None,
            max_width: None,
            max_height: None,
            backend: None,
        },
        None,
        None,
        None,
    )?;
    let resolved_points = detection_response
        .detected_points()
        .map(|points| points.to_vec());
    Ok((
        "native-ORT",
        None,
        resolved_points,
        Some(detection_response),
    ))
}

fn resolve_points_sidecar_path(image_path: &Path, explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path.to_path_buf());
    }

    let inline_sidecar = PathBuf::from(format!("{}.points.json", image_path.display()));
    if inline_sidecar.exists() {
        return Some(inline_sidecar);
    }

    let stem_sidecar = image_path
        .file_stem()
        .map(|stem| image_path.with_file_name(format!("{}.points.json", stem.to_string_lossy())));
    stem_sidecar.filter(|path| path.exists())
}

fn read_sidecar_points(path: &Path) -> Result<Vec<ScannerPoint>, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read points sidecar {}: {error}", path.display()))?;
    let parsed: SidecarPointsFile = serde_json::from_str(&content)
        .map_err(|error| format!("Failed to parse points sidecar {}: {error}", path.display()))?;
    let points = parsed.into_points();
    if points.len() != 4 {
        return Err(format!(
            "Points sidecar {} must contain exactly 4 points, got {}.",
            path.display(),
            points.len()
        ));
    }
    Ok(points)
}

fn write_validation_metadata(path: &Path, metadata: &ValidationMetadata) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(metadata)
        .map_err(|error| format!("Failed to serialize validation metadata: {error}"))?;
    fs::write(path, serialized).map_err(|error| {
        format!(
            "Failed to write validation metadata {}: {error}",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sidecar_points_from_array() {
        let parsed: SidecarPointsFile = serde_json::from_str(
            r#"[{"x":1.0,"y":2.0},{"x":3.0,"y":4.0},{"x":5.0,"y":6.0},{"x":7.0,"y":8.0}]"#,
        )
        .expect("sidecar array should parse");
        assert_eq!(parsed.into_points().len(), 4);
    }

    #[test]
    fn parses_sidecar_points_from_document_points_object() {
        let parsed: SidecarPointsFile = serde_json::from_str(
            r#"{"documentPoints":[{"x":1.0,"y":2.0},{"x":3.0,"y":4.0},{"x":5.0,"y":6.0},{"x":7.0,"y":8.0}]}"#,
        )
        .expect("sidecar object should parse");
        assert_eq!(parsed.into_points().len(), 4);
    }
}
