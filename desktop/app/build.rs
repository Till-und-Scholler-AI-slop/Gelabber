use std::path::{Path, PathBuf};

include!("src/commands.rs");

/// Windows has no rpath: the loader takes gelabber_media.dll from the
/// binary's own directory. Copies it from the core's library directory to
/// where this build puts the binary (`target/<profile>/`), for running the
/// build, `cargo test` and the smoke test.
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
    // Find the native core during development and tests; packages ship it
    // next to the binary ($ORIGIN on Linux, the loader's rule on Windows).
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
    // `allow-<command>` permissions for the app's own commands; the server
    // origin gets the `media` set (permissions/media.toml) plus
    // `allow-open-setup`, the bundled setup page only `allow-set-server`
    // (capabilities/setup.json).
    let commands: Vec<&'static str> = MEDIA_COMMANDS
        .iter()
        .copied()
        .chain(["set_server", "open_setup"])
        .collect();
    let commands: &'static [&'static str] = Box::leak(commands.into_boxed_slice());
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(commands)),
    )
    .expect("tauri build");
}
