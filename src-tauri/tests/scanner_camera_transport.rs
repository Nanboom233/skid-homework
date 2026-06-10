#![allow(dead_code)]

#[path = "../src/adb_plugin.rs"]
mod adb_plugin;
#[path = "../src/scanner_assets.rs"]
mod scanner_assets;
#[path = "../src/scanner_camera_resource.rs"]
mod scanner_camera_resource;
#[path = "../src/scanner_frame_protocol.rs"]
mod scanner_frame_protocol;
#[path = "../src/scanner_platform.rs"]
mod scanner_platform;
#[path = "../src/scanner_resource.rs"]
mod scanner_resource;
#[path = "../src/scanner_transport.rs"]
mod scanner_transport;

use scanner_frame_protocol::{
    decode_i420_payload_to_rgb_image, parse_frame_packet_header, write_frame_telemetry,
    FRAME_CODEC_I420, FRAME_CODEC_I420_TELEMETRY, FRAME_PACKET_HEADER_SIZE,
    FRAME_PACKET_TELEMETRY_SIZE,
};

#[test]
fn parses_i420_telemetry_frame_packet_header() {
    let mut packet = Vec::new();
    packet.push(FRAME_CODEC_I420_TELEMETRY);
    packet.extend_from_slice(&2u32.to_be_bytes());
    packet.extend_from_slice(&2u32.to_be_bytes());
    packet.extend_from_slice(&[0u8; FRAME_PACKET_TELEMETRY_SIZE]);
    packet.extend_from_slice(&[16, 16, 16, 16, 128, 128]);

    let (width, height, payload) = parse_frame_packet_header(&packet).unwrap();

    assert_eq!(width, 2);
    assert_eq!(height, 2);
    assert_eq!(payload, &[16, 16, 16, 16, 128, 128]);
}

#[test]
fn parses_i420_frame_packet_header_without_telemetry() {
    let mut packet = Vec::new();
    packet.push(FRAME_CODEC_I420);
    packet.extend_from_slice(&2u32.to_be_bytes());
    packet.extend_from_slice(&2u32.to_be_bytes());
    packet.extend_from_slice(&[16, 16, 16, 16, 128, 128]);

    let (width, height, payload) = parse_frame_packet_header(&packet).unwrap();

    assert_eq!(width, 2);
    assert_eq!(height, 2);
    assert_eq!(payload, &[16, 16, 16, 16, 128, 128]);
}

#[test]
fn writes_and_parses_i420_telemetry_round_trip() {
    let timestamp = 1_786_000_000_123u64;
    let sequence = 42u32;
    let payload_bytes = [16, 16, 16, 16, 128, 128];
    let mut packet = Vec::new();
    packet.push(FRAME_CODEC_I420_TELEMETRY);
    packet.extend_from_slice(&2u32.to_be_bytes());
    packet.extend_from_slice(&2u32.to_be_bytes());
    packet.extend_from_slice(&[0u8; FRAME_PACKET_TELEMETRY_SIZE]);
    packet.extend_from_slice(&payload_bytes);

    write_frame_telemetry(&mut packet, timestamp, sequence).unwrap();
    let (width, height, payload) = parse_frame_packet_header(&packet).unwrap();

    assert_eq!(width, 2);
    assert_eq!(height, 2);
    assert_eq!(payload, payload_bytes);

    let telemetry_start = FRAME_PACKET_HEADER_SIZE;
    let telemetry_end = FRAME_PACKET_HEADER_SIZE + FRAME_PACKET_TELEMETRY_SIZE;
    let telemetry = &packet[telemetry_start..telemetry_end];
    assert_eq!(&telemetry[..8], timestamp.to_be_bytes());
    assert_eq!(&telemetry[8..], sequence.to_be_bytes());
}

#[test]
fn rejects_invalid_preview_frame_packets() {
    let short_packet = [FRAME_CODEC_I420; FRAME_PACKET_HEADER_SIZE - 1];
    assert!(parse_frame_packet_header(&short_packet).is_err());

    let mut invalid_codec_packet = vec![99];
    invalid_codec_packet.extend_from_slice(&2u32.to_be_bytes());
    invalid_codec_packet.extend_from_slice(&2u32.to_be_bytes());
    assert!(parse_frame_packet_header(&invalid_codec_packet).is_err());

    let mut truncated_telemetry_packet = vec![FRAME_CODEC_I420_TELEMETRY];
    truncated_telemetry_packet.extend_from_slice(&2u32.to_be_bytes());
    truncated_telemetry_packet.extend_from_slice(&2u32.to_be_bytes());
    assert!(parse_frame_packet_header(&truncated_telemetry_packet).is_err());
}

#[test]
fn validates_i420_payload_shape() {
    let payload = [16, 16, 16, 16, 128, 128];
    let image = decode_i420_payload_to_rgb_image(&payload, 2, 2).unwrap();
    assert_eq!(image.width(), 2);
    assert_eq!(image.height(), 2);

    assert!(decode_i420_payload_to_rgb_image(&payload, 3, 2).is_err());
    assert!(decode_i420_payload_to_rgb_image(&payload[..5], 2, 2).is_err());
}

#[test]
fn decodes_i420_payload_to_expected_rgb() {
    let payload = [50, 80, 120, 200, 128, 128];
    let image = decode_i420_payload_to_rgb_image(&payload, 2, 2).unwrap();
    assert_eq!(image.width(), 2);
    assert_eq!(image.height(), 2);

    let pixels = [
        image.get_pixel(0, 0).0,
        image.get_pixel(1, 0).0,
        image.get_pixel(0, 1).0,
        image.get_pixel(1, 1).0,
    ];

    for pixel in &pixels {
        let r = pixel[0] as i16;
        let g = pixel[1] as i16;
        let b = pixel[2] as i16;
        assert!((r - g).abs() <= 1, "R and G differ too much: {pixel:?}");
        assert!((g - b).abs() <= 1, "G and B differ too much: {pixel:?}");
    }

    assert!(pixels[0][0] < pixels[1][0]);
    assert!(pixels[1][0] < pixels[2][0]);
    assert!(pixels[2][0] < pixels[3][0]);
}

#[test]
fn validates_still_capture_jpeg_payloads() {
    let jpeg = vec![0xff, 0xd8, 0xff, 0xd9];
    assert_eq!(
        scanner_transport::validate_still_capture_payload("serial", jpeg.clone(), "Still").unwrap(),
        jpeg
    );

    assert!(scanner_transport::validate_still_capture_payload("serial", vec![], "Still").is_err());
    assert!(scanner_transport::validate_still_capture_payload(
        "serial",
        b"camera error".to_vec(),
        "Still",
    )
    .unwrap_err()
    .contains("not a JPEG"));
    assert!(scanner_transport::validate_still_capture_payload(
        "serial",
        vec![0xff, 0xd8, 0x00],
        "Still",
    )
    .unwrap_err()
    .contains("truncated JPEG"));

    let binary_error = scanner_transport::validate_still_capture_payload(
        "serial",
        vec![0x00, 0x01, 0x02, 0xff, 0x00, 0x10, 0x11, 0x12],
        "Still",
    )
    .unwrap_err();
    assert!(binary_error.contains("not a JPEG"));
    assert!(binary_error.contains("len=8"));
    assert!(binary_error.contains("head=[00 01 02 ff 00 10 11 12]"));
    assert!(binary_error.contains("tail=[00 01 02 ff 00 10 11 12]"));
    assert!(binary_error.contains("first_soi=None"));
    assert!(binary_error.contains("last_eoi=None"));
}

#[test]
fn resolves_bundled_camera_server_resource() {
    let temp = tempfile::tempdir().unwrap();
    let resource_path = temp.path().join("camera-server.jar");
    std::fs::write(&resource_path, b"jar").unwrap();

    let artifact = scanner_camera_resource::resolve_camera_server_artifact_from_resource_dir(Some(
        temp.path().to_path_buf(),
    ))
    .unwrap();

    assert_eq!(artifact.source, "tauri-resource-dir");
    assert_eq!(
        artifact.path,
        resource_path.canonicalize().unwrap().display().to_string()
    );
}

#[test]
fn installed_camera_server_asset_takes_precedence_over_bundled_resource() {
    let installed = tempfile::tempdir().unwrap();
    let bundled = tempfile::tempdir().unwrap();
    let installed_path = installed.path().join("camera-server.jar");
    let bundled_path = bundled.path().join("camera-server.jar");
    std::fs::write(&installed_path, b"installed").unwrap();
    std::fs::write(&bundled_path, b"bundled").unwrap();

    let artifact = scanner_camera_resource::resolve_camera_server_artifact_from_candidates(
        Some(installed_path.clone()),
        Some(bundled.path().to_path_buf()),
    )
    .unwrap();

    assert_eq!(artifact.source, "scanner-camera-asset");
    assert_eq!(
        artifact.path,
        installed_path.canonicalize().unwrap().display().to_string()
    );
}

#[test]
fn fails_when_resource_dir_is_unavailable() {
    let error = scanner_camera_resource::resolve_camera_server_artifact_from_resource_dir(None)
        .unwrap_err();

    assert!(
        error.contains("resource directory is unavailable"),
        "unexpected error message: {error}"
    );
}

#[test]
fn fails_when_bundled_camera_server_jar_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    let error = scanner_camera_resource::resolve_camera_server_artifact_from_resource_dir(Some(
        temp.path().to_path_buf(),
    ))
    .unwrap_err();

    assert!(
        error.contains("Bundled camera-server.jar was not found"),
        "unexpected error message: {error}"
    );
}
