// Bettet das Manifest ein: moderne Steuerelemente (Common Controls 6) und DPI-Bewusstsein.
// Nur für den MSVC-Linker; der GNU-Build (lokale Prüfung unter Linux) läuft ohne.
fn main() {
    println!("cargo:rerun-if-changed=hikidashi.manifest");
    let ziel = std::env::var("TARGET").unwrap_or_default();
    if ziel.ends_with("windows-msvc") {
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("hikidashi.manifest");
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
}
