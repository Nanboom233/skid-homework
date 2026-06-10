#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(any(target_os = "windows", target_os = "linux"))]
#[allow(dead_code)]
#[path = "../scanner_ort_protocol.rs"]
mod scanner_ort_protocol;
#[cfg(any(target_os = "windows", target_os = "linux"))]
#[path = "../scanner_ort_worker_impl.rs"]
mod scanner_ort_worker_impl;
#[cfg(any(target_os = "windows", target_os = "linux"))]
#[allow(dead_code)]
#[path = "../scanner_platform.rs"]
mod scanner_platform;
#[cfg(any(target_os = "windows", target_os = "linux"))]
#[allow(dead_code)]
#[path = "../scanner_resource.rs"]
mod scanner_resource;

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn main() {
    if let Err(error) = run() {
        eprintln!("scanner ORT worker failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn run() -> std::io::Result<()> {
    use std::io::{BufReader, BufWriter};

    use scanner_ort_protocol::{read_framed_json, write_framed_json, ScannerOrtWorkerRequest};

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());

    while let Some(request) = read_framed_json::<_, ScannerOrtWorkerRequest>(&mut reader)? {
        let (response, should_exit) = scanner_ort_worker_impl::handle_worker_request(request);
        write_framed_json(&mut writer, &response)?;
        if should_exit {
            break;
        }
    }

    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn main() {
    eprintln!("scanner ORT worker is only supported on Windows and Linux desktop targets.");
    std::process::exit(1);
}
