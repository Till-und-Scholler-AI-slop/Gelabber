//! Links the native media core (desktop/native).
//!
//! - `GELABBER_MEDIA_LIB_DIR`: directory with a prebuilt `libgelabber_media`
//!   (packaging, or reuse between builds).
//! - otherwise `GELABBER_LIBWEBRTC_DIR`: libwebrtc package from
//!   `desktop/native/scripts/build-libwebrtc-linux.sh`; the core is built here
//!   with CMake and the package's own toolchain.
use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=GELABBER_MEDIA_LIB_DIR");
    println!("cargo:rerun-if-env-changed=GELABBER_LIBWEBRTC_DIR");
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
            .define("GELABBER_LIBWEBRTC_DIR", &webrtc)
            .profile("Release")
            .generator("Ninja")
            // Only the core: libsdptransform's tests and helpers are part of
            // `all` and cannot link without libwebrtc's libc++.
            .build_target("gelabber_media");
        if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
            config.define(
                "CMAKE_TOOLCHAIN_FILE",
                native.join("cmake/chromium-linux.cmake"),
            );
        }
        config.build().join("build")
    };

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=gelabber_media");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        // Development and tests; packaged apps ship the library next to the binary.
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
    }
}
