include!("src/commands.rs");

fn main() {
    // Find the native core during development and tests; packages ship it
    // next to the binary ($ORIGIN).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        if let Ok(dir) = std::env::var("DEP_GELABBER_MEDIA_LIB_DIR") {
            println!("cargo:rustc-link-arg-bins=-Wl,-rpath,{dir}");
        }
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
    }
    // `allow-<command>` permissions for the app's own commands; the server
    // origin gets the `media` set (permissions/media.toml), the bundled setup
    // page only `allow-set-server` (capabilities/setup.json).
    let commands: Vec<&'static str> = MEDIA_COMMANDS
        .iter()
        .copied()
        .chain(["set_server"])
        .collect();
    let commands: &'static [&'static str] = Box::leak(commands.into_boxed_slice());
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(commands)),
    )
    .expect("tauri build");
}
