use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

enum OpenCvLinkConfig {
    Windows {
        lib_dir: PathBuf,
    },
    System,
    Android {
        abi: &'static str,
        compat_lib_dir: Option<PathBuf>,
        cxx_static_lib: PathBuf,
        cxx_abi_lib: PathBuf,
        static_lib_dir: PathBuf,
        third_party_lib_dir: PathBuf,
    },
}

fn main() {
    println!("cargo:rerun-if-changed=native/scan_enhance.cpp");
    println!("cargo:rerun-if-env-changed=OPENCV_WINDOWS_SDK_DIR");
    println!("cargo:rerun-if-env-changed=OPENCV_ANDROID_SDK_DIR");

    let target = env::var("TARGET").unwrap_or_default();
    let mut build = cc::Build::new();
    build.cpp(true).std("c++17").file("native/scan_enhance.cpp");

    let link_config = if target.contains("android") {
        configure_android_opencv(&mut build, &target)
    } else if target.contains("windows") {
        configure_windows_opencv(&mut build)
    } else if target.contains("linux") || target.contains("apple-darwin") {
        configure_system_opencv(&mut build, &target)
    } else {
        panic!("scan enhancement requires a prebuilt OpenCV SDK for target {target}");
    };

    build.compile("skid_scan_enhance_native");
    link_config.emit();

    tauri_build::build()
}

impl OpenCvLinkConfig {
    fn emit(&self) {
        match self {
            Self::System => {}
            Self::Windows { lib_dir } => {
                println!("cargo:rustc-link-search=native={}", lib_dir.display());
                println!("cargo:rustc-link-lib=dylib=opencv_world4100");
            }
            Self::Android {
                abi,
                compat_lib_dir,
                cxx_static_lib,
                cxx_abi_lib,
                static_lib_dir,
                third_party_lib_dir,
            } => {
                println!(
                    "cargo:rustc-link-search=native={}",
                    static_lib_dir.display()
                );
                if let Some(compat_lib_dir) = compat_lib_dir {
                    println!(
                        "cargo:rustc-link-search=native={}",
                        compat_lib_dir.display()
                    );
                }
                println!(
                    "cargo:rustc-link-search=native={}",
                    third_party_lib_dir.display()
                );

                for lib in ["opencv_imgcodecs", "opencv_imgproc", "opencv_core"] {
                    println!("cargo:rustc-link-lib=static={lib}");
                }

                for lib in android_third_party_libs(abi) {
                    println!("cargo:rustc-link-lib=static={lib}");
                }

                for lib in ["z", "dl", "m", "log"] {
                    println!("cargo:rustc-link-lib={lib}");
                }
                println!("cargo:rustc-link-arg={}", cxx_static_lib.display());
                println!("cargo:rustc-link-arg={}", cxx_abi_lib.display());
            }
        }
    }
}

fn configure_windows_opencv(build: &mut cc::Build) -> OpenCvLinkConfig {
    build.flag_if_supported("/EHsc");

    let sdk_dir = env_path_or_default(
        "OPENCV_WINDOWS_SDK_DIR",
        default_home_path([".local", "opencv", "4.10.0", "opencv", "build"]),
    );
    let include_dir = sdk_dir.join("include");
    let lib_dir = sdk_dir.join("x64").join("vc16").join("lib");

    require_dir(&include_dir, "OpenCV Windows include directory");
    require_dir(&lib_dir, "OpenCV Windows library directory");

    build.include(include_dir);
    OpenCvLinkConfig::Windows { lib_dir }
}

fn configure_system_opencv(build: &mut cc::Build, target: &str) -> OpenCvLinkConfig {
    let library = pkg_config::Config::new()
        .atleast_version("4")
        .probe("opencv4")
        .unwrap_or_else(|error| {
            panic!(
                "OpenCV 4 pkg-config metadata is required for target {target}. \
                 Install libopencv-dev on Linux or opencv/pkgconf on macOS, \
                 then set PKG_CONFIG_PATH if pkg-config cannot find opencv4.pc: {error}"
            )
        });

    for include_path in library.include_paths {
        build.include(include_path);
    }

    OpenCvLinkConfig::System
}

fn configure_android_opencv(build: &mut cc::Build, target: &str) -> OpenCvLinkConfig {
    build
        .flag_if_supported("-fexceptions")
        .flag_if_supported("-frtti")
        .cpp_link_stdlib(None::<&str>);

    let abi = android_abi(target);
    let cxx_lib_dir = android_cxx_lib_dir(target);
    let cxx_static_lib = cxx_lib_dir.join("libc++_static.a");
    let cxx_abi_lib = cxx_lib_dir.join("libc++abi.a");
    let sdk_dir = env_path_or_default(
        "OPENCV_ANDROID_SDK_DIR",
        default_home_path([".local", "opencv", "4.10.0-android", "OpenCV-android-sdk"]),
    );
    let native_dir = sdk_dir.join("sdk").join("native");
    let include_dir = native_dir.join("jni").join("include");
    let static_lib_dir = native_dir.join("staticlibs").join(abi);
    let third_party_lib_dir = native_dir.join("3rdparty").join("libs").join(abi);
    let compat_lib_dir = prepare_android_compat_libs(abi, &third_party_lib_dir);

    require_dir(&include_dir, "OpenCV Android include directory");
    require_file(&cxx_static_lib, "Android NDK static C++ runtime");
    require_file(&cxx_abi_lib, "Android NDK C++ ABI runtime");
    require_dir(&static_lib_dir, "OpenCV Android static library directory");
    require_dir(
        &third_party_lib_dir,
        "OpenCV Android third-party library directory",
    );

    build.include(include_dir);
    OpenCvLinkConfig::Android {
        abi,
        compat_lib_dir,
        cxx_static_lib,
        cxx_abi_lib,
        static_lib_dir,
        third_party_lib_dir,
    }
}

fn env_path_or_default(name: &str, default: PathBuf) -> PathBuf {
    env::var_os(name).map(PathBuf::from).unwrap_or(default)
}

fn default_home_path<const N: usize>(segments: [&str; N]) -> PathBuf {
    let home = env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .unwrap_or_else(|| OsString::from("."));
    segments
        .iter()
        .fold(PathBuf::from(home), |path, segment| path.join(segment))
}

fn require_dir(path: &Path, label: &str) {
    if !path.is_dir() {
        panic!("{label} not found: {}", path.display());
    }
}

fn require_file(path: &Path, label: &str) {
    if !path.is_file() {
        panic!("{label} not found: {}", path.display());
    }
}

fn android_abi(target: &str) -> &'static str {
    if target.starts_with("aarch64-linux-android") {
        "arm64-v8a"
    } else if target.starts_with("armv7-linux-androideabi") {
        "armeabi-v7a"
    } else if target.starts_with("i686-linux-android") {
        "x86"
    } else if target.starts_with("x86_64-linux-android") {
        "x86_64"
    } else {
        panic!("unsupported Android target for OpenCV scan enhancement: {target}");
    }
}

fn android_cxx_lib_dir(target: &str) -> PathBuf {
    android_ndk_dir()
        .join("toolchains")
        .join("llvm")
        .join("prebuilt")
        .join(android_ndk_host_tag())
        .join("sysroot")
        .join("usr")
        .join("lib")
        .join(android_ndk_lib_triple(target))
}

fn prepare_android_compat_libs(abi: &str, third_party_lib_dir: &Path) -> Option<PathBuf> {
    if abi != "x86" && abi != "x86_64" {
        return None;
    }

    let source = third_party_lib_dir.join("libippicv.a");
    require_file(&source, "OpenCV Android IPP archive");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"))
        .join(format!("opencv-android-{abi}-compat"));
    std::fs::create_dir_all(&out_dir).unwrap_or_else(|error| {
        panic!(
            "failed to create OpenCV Android compat directory {}: {error}",
            out_dir.display()
        )
    });

    let output = out_dir.join("libippicv.a");
    let status = Command::new(android_ndk_tool("llvm-objcopy"))
        .arg("--remove-section=.note.gnu.property")
        .arg(&source)
        .arg(&output)
        .status()
        .unwrap_or_else(|error| panic!("failed to run llvm-objcopy for OpenCV IPP: {error}"));
    if !status.success() {
        panic!("llvm-objcopy failed while preparing OpenCV IPP archive: {status}");
    }

    require_file(&output, "OpenCV Android compatible IPP archive");
    Some(out_dir)
}

fn android_ndk_tool(tool_name: &str) -> PathBuf {
    let executable = if cfg!(target_os = "windows") {
        format!("{tool_name}.exe")
    } else {
        tool_name.to_string()
    };
    android_ndk_dir()
        .join("toolchains")
        .join("llvm")
        .join("prebuilt")
        .join(android_ndk_host_tag())
        .join("bin")
        .join(executable)
}

fn android_ndk_dir() -> PathBuf {
    env::var_os("ANDROID_NDK_HOME")
        .or_else(|| env::var_os("ANDROID_NDK_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("ANDROID_NDK_HOME is required for Android OpenCV linking"))
}

fn android_ndk_host_tag() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows-x86_64"
    } else if cfg!(target_os = "macos") {
        "darwin-x86_64"
    } else {
        "linux-x86_64"
    }
}

fn android_ndk_lib_triple(target: &str) -> &'static str {
    if target.starts_with("aarch64-linux-android") {
        "aarch64-linux-android"
    } else if target.starts_with("armv7-linux-androideabi") {
        "arm-linux-androideabi"
    } else if target.starts_with("i686-linux-android") {
        "i686-linux-android"
    } else if target.starts_with("x86_64-linux-android") {
        "x86_64-linux-android"
    } else {
        panic!("unsupported Android target for NDK C++ runtime: {target}");
    }
}

fn android_third_party_libs(abi: &str) -> &'static [&'static str] {
    match abi {
        "arm64-v8a" | "armeabi-v7a" => &[
            "libjpeg-turbo",
            "libwebp",
            "libpng",
            "libtiff",
            "libopenjp2",
            "IlmImf",
            "tbb",
            "ittnotify",
            "cpufeatures",
            "tegra_hal",
        ],
        "x86" | "x86_64" => &[
            "libjpeg-turbo",
            "libwebp",
            "libpng",
            "libtiff",
            "libopenjp2",
            "IlmImf",
            "tbb",
            "ittnotify",
            "cpufeatures",
            "ippiw",
            "ippicv",
        ],
        _ => panic!("unsupported Android ABI for OpenCV scan enhancement: {abi}"),
    }
}
