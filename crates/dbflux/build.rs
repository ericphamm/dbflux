//! Build script for the `dbflux` binary.
//!
//! Its only job is embedding the application icon into `dbflux.exe`. Windows
//! reads an executable's icon from an embedded resource, so the `.ico` the
//! installer and the Start-menu shortcuts use does not cover the binary
//! itself — without this, `dbflux.exe` is generic in Explorer, on the taskbar,
//! and in the Open With dialog.
//!
//! Nothing here runs on macOS or Linux: those platforms take the icon from the
//! bundle and from the desktop entry instead.

fn main() {
    #[cfg(windows)]
    embed_windows_icon();
}

/// Compile a one-line resource script naming the icon, and link it in.
///
/// The script is generated into `OUT_DIR` with an absolute path rather than
/// committed next to the crate, so the icon stays a single file at the repo
/// root instead of being duplicated per target.
#[cfg(windows)]
fn embed_windows_icon() {
    use std::path::PathBuf;

    // Both release channels ship the same artwork, so the executable takes
    // stable's icon. The channel affects the bundle identity, not the mark.
    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR is set for every build script"),
    );
    let icon = manifest_dir.join("../../resources/branding/stable/icon.ico");
    let icon = icon.canonicalize().unwrap_or_else(|error| {
        panic!("application icon not found at {}: {error}", icon.display())
    });

    println!("cargo:rerun-if-changed={}", icon.display());

    // Resource scripts take C-style string literals, so a Windows path's
    // backslashes have to be escaped.
    let icon_literal = icon.display().to_string().replace('\\', "\\\\");

    let out_dir =
        PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set for every build script"));
    let script = out_dir.join("dbflux-icon.rc");

    // Resource id 1 is what the shell picks up as the executable's icon. gpui
    // embeds its manifest under the same id but a different resource type, so
    // the two do not collide.
    std::fs::write(&script, format!("1 ICON \"{icon_literal}\"\n"))
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", script.display()));

    // `manifest_optional` tolerates an environment with no resource compiler
    // and fails only when compilation was attempted and did not work. A build
    // that silently produced an icon-less executable would be worse.
    if let Err(error) = embed_resource::compile(&script, embed_resource::NONE).manifest_optional() {
        panic!("failed to embed the application icon: {error}");
    }
}
