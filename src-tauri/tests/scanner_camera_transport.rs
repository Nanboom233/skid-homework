#![allow(dead_code)]

#[path = "../src/adb_plugin.rs"]
mod adb_plugin;
#[path = "../src/scanner_camera_resource.rs"]
mod scanner_camera_resource;
#[path = "../src/scanner_frame_protocol.rs"]
mod scanner_frame_protocol;
#[path = "../src/scanner_resource.rs"]
mod scanner_resource;
#[path = "../src/scanner_transport.rs"]
mod scanner_transport;

use scanner_frame_protocol::{
    decode_i420_payload_to_rgb_image, parse_frame_packet_header, FRAME_CODEC_I420,
    FRAME_CODEC_I420_TELEMETRY, FRAME_PACKET_HEADER_SIZE, FRAME_PACKET_TELEMETRY_SIZE,
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
