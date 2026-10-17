// The adapter's release identity must not depend on its consumer's Git checkout.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!(
        "cargo:rustc-env=GIT_VERSION=ironcalc-0.8.3+jade-{}",
        env!("CARGO_PKG_VERSION")
    );
}
