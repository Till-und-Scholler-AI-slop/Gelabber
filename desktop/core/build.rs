//! Links the native media core (desktop/native).
//!
//! - `GELABBER_MEDIA_LIB_DIR`: directory with a prebuilt core (packaging, or
//!   reuse between builds): `libgelabber_media.so` on Linux; on Windows
//!   `gelabber_media.lib` (the import library) next to `gelabber_media.dll`.
//! - otherwise `GELABBER_LIBWEBRTC_DIR`: libwebrtc package from
//!   `desktop/native/scripts/build-libwebrtc-linux.sh` or
//!   `build-libwebrtc-windows.ps1`; the core is built here with CMake. On
//!   Linux the package brings its own toolchain. On Windows the compiler is
//!   `clang-cl` from `PATH`, which built the package; `GELABBER_MEDIA_COMPILER`
//!   names another one (`cl`, or a path).
//!
//! Windows has no rpath: the loader takes `gelabber_media.dll` from the
//! executable's directory or `PATH`. Cargo puts the directory of a core built
//! here on `PATH` for `cargo run` and `cargo test`; everything else (a
//! prebuilt core, a packaged app) needs the DLL next to the executable.
use std::{env, path::PathBuf};

/// A path as a CMake `-D` value: CMake reads `\` as an escape once the value
/// reaches a generated file.
fn cmake_path(path: &str) -> String {
    if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        path.to_owned()
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=GELABBER_MEDIA_LIB_DIR");
    println!("cargo:rerun-if-env-changed=GELABBER_LIBWEBRTC_DIR");
    println!("cargo:rerun-if-env-changed=GELABBER_MEDIA_COMPILER");
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let native = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../native");
    for path in [
        "CMakeLists.txt",
        "libwebrtc.env",
        "gelabber_media.map",
        "include",
        "src",
        "cmake",
    ] {
        println!("cargo:rerun-if-changed={}", native.join(path).display());
    }

    let lib_dir = if let Ok(dir) = env::var("GELABBER_MEDIA_LIB_DIR") {
        PathBuf::from(dir)
    } else {
        let webrtc = env::var("GELABBER_LIBWEBRTC_DIR").expect(
            "set GELABBER_LIBWEBRTC_DIR (libwebrtc package) or GELABBER_MEDIA_LIB_DIR (prebuilt core)",
        );
        let mut config = cmake::Config::new(&native);
        config
            .define("GELABBER_LIBWEBRTC_DIR", cmake_path(&webrtc))
            .profile("Release")
            .generator("Ninja")
            // Only the core: libsdptransform's tests and helpers are part of
            // `all` and cannot link without libwebrtc's libc++.
            .build_target("gelabber_media");
        match target_os.as_str() {
            "linux" => {
                config.define(
                    "CMAKE_TOOLCHAIN_FILE",
                    native.join("cmake/chromium-linux.cmake"),
                );
            }
            "windows" => {
                // libwebrtc is built against the static CRT (/MT); the cmake
                // crate would pass Rust's /MD. The DLL's C ABI keeps the two
                // runtimes apart.
                config.static_crt(true);
                let compiler =
                    env::var("GELABBER_MEDIA_COMPILER").unwrap_or_else(|_| "clang-cl".to_owned());
                let compiler = cmake_path(&compiler);
                config
                    .define("CMAKE_C_COMPILER", &compiler)
                    .define("CMAKE_CXX_COMPILER", &compiler);
            }
            _ => {}
        }
        config.build().join("build")
    };

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    // For dependents (the desktop app's build script, as
    // DEP_GELABBER_MEDIA_LIB_DIR): the rpath on Linux; on Windows the
    // directory that holds gelabber_media.dll, to copy next to the executable.
    println!("cargo:lib_dir={}", lib_dir.display());
    // On Windows this links gelabber_media.lib, the DLL's import library.
    println!("cargo:rustc-link-lib=dylib=gelabber_media");
    if target_os == "linux" {
        // Development and tests; packaged apps ship the library next to the binary.
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
    }
}
