fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() == Some("windows") {
        // Skia's ICU time-zone implementation reads the Windows registry.
        println!("cargo:rustc-link-lib=advapi32");
    }
}
