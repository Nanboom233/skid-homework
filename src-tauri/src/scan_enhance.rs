use std::{ptr, slice};

use tauri::http::{header, Method, Request, Response, StatusCode};
use thiserror::Error;

const ENHANCE_SCAN_PATH: &str = "/enhance_scan";
const NATIVE_STATUS_OK: i32 = 0;
const NATIVE_STATUS_EMPTY_INPUT: i32 = 1;
const NATIVE_STATUS_INVALID_IMAGE: i32 = 2;
const NATIVE_STATUS_PROCESSING_FAILED: i32 = 3;
const NATIVE_STATUS_PNG_ENCODE_FAILED: i32 = 4;
const NATIVE_STATUS_OUT_OF_MEMORY: i32 = 5;

extern "C" {
    fn skid_enhance_scan(
        input: *const u8,
        input_len: usize,
        output: *mut *mut u8,
        output_len: *mut usize,
    ) -> i32;
    fn skid_enhance_scan_free(ptr: *mut u8);
}

#[derive(Debug, Error)]
pub enum ScanEnhanceError {
    #[error("Image enhancement request body is empty.")]
    EmptyInput,
    #[error("Image enhancement request body is not a supported image.")]
    InvalidImage,
    #[error("OpenCV scan enhancement failed.")]
    ProcessingFailed,
    #[error("OpenCV scan enhancement failed to encode PNG output.")]
    PngEncodeFailed,
    #[error("OpenCV scan enhancement ran out of memory.")]
    OutOfMemory,
}

impl ScanEnhanceError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::EmptyInput | Self::InvalidImage => StatusCode::BAD_REQUEST,
            Self::ProcessingFailed | Self::PngEncodeFailed | Self::OutOfMemory => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

pub fn enhance_scan(source_bytes: &[u8]) -> Result<Vec<u8>, ScanEnhanceError> {
    if source_bytes.is_empty() {
        return Err(ScanEnhanceError::EmptyInput);
    }

    let mut output = ptr::null_mut();
    let mut output_len = 0usize;
    let status = unsafe {
        skid_enhance_scan(
            source_bytes.as_ptr(),
            source_bytes.len(),
            &mut output,
            &mut output_len,
        )
    };

    if status != NATIVE_STATUS_OK {
        if !output.is_null() {
            unsafe { skid_enhance_scan_free(output) };
        }
        return match status {
            NATIVE_STATUS_EMPTY_INPUT => Err(ScanEnhanceError::EmptyInput),
            NATIVE_STATUS_INVALID_IMAGE => Err(ScanEnhanceError::InvalidImage),
            NATIVE_STATUS_PNG_ENCODE_FAILED => Err(ScanEnhanceError::PngEncodeFailed),
            NATIVE_STATUS_OUT_OF_MEMORY => Err(ScanEnhanceError::OutOfMemory),
            NATIVE_STATUS_PROCESSING_FAILED => Err(ScanEnhanceError::ProcessingFailed),
            _ => Err(ScanEnhanceError::ProcessingFailed),
        };
    }

    if output.is_null() || output_len == 0 {
        if !output.is_null() {
            unsafe { skid_enhance_scan_free(output) };
        }
        return Err(ScanEnhanceError::PngEncodeFailed);
    }

    let bytes = unsafe { slice::from_raw_parts(output, output_len).to_vec() };
    unsafe { skid_enhance_scan_free(output) };
    Ok(bytes)
}

pub fn handle_protocol(request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    if request.uri().path() != ENHANCE_SCAN_PATH {
        return text_response(StatusCode::NOT_FOUND, "Not found.");
    }

    if request.method() != Method::POST {
        return text_response(StatusCode::METHOD_NOT_ALLOWED, "Method not allowed.");
    }

    match enhance_scan(request.body()) {
        Ok(bytes) => binary_response(StatusCode::OK, "image/png", bytes),
        Err(error) => text_response(error.status_code(), &error.to_string()),
    }
}

fn binary_response(status: StatusCode, content_type: &str, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .body(body)
        .expect("valid scan enhancement binary response")
}

fn text_response(status: StatusCode, body: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(body.as_bytes().to_vec())
        .expect("valid scan enhancement text response")
}
