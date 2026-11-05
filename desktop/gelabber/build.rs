//! Lets the binary find the native media core: on Linux through the rpath
//! (`$ORIGIN` for the packages, the core's build directory for `cargo run`),
//! on Windows by copying `gelabber_media.dll` next to the executable, which
//! has no rpath (as desktop/app/build.rs does for the Tauri app).
use std::path::{Path, PathBuf};

fn place_core_dll(lib_dir: &Path) {
    let dll = lib_dir.join("gelabber_media.dll");
    println!("cargo:rerun-if-changed={}", dll.display());
    // OUT_DIR is `<profile dir>/build/<package>-<hash>/out`, on newer cargo
    // `<profile dir>/build/<package>/<hash>/out`.
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let profile_dir = out_dir
        .ancestors()
        .find(|dir| dir.file_name().is_some_and(|name| name == "build"))
        .and_then(Path::parent);
    match profile_dir {
        Some(profile_dir) if dll.is_file() => {
            std::fs::copy(&dll, profile_dir.join("gelabber_media.dll"))
                .expect("copy gelabber_media.dll next to the binary");
        }
        // Type checks (`cargo check`, clippy) run without the core.
        _ => println!(
            "cargo:warning=no gelabber_media.dll in {}: the built app will not start",
            lib_dir.display()
        ),
    }
}

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS");
    let lib_dir = std::env::var("DEP_GELABBER_MEDIA_LIB_DIR");
    if target_os.as_deref() == Ok("linux") {
        if let Ok(dir) = &lib_dir {
            println!("cargo:rustc-link-arg-bins=-Wl,-rpath,{dir}");
        }
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
    }
    if target_os.as_deref() == Ok("windows")
        && let Ok(dir) = &lib_dir
    {
        place_core_dll(Path::new(dir));
    }
}
