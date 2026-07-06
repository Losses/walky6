fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let dist_dir = std::path::Path::new(&manifest_dir).join("..").join("dist");
    println!("cargo:rustc-env=WALKY6_DIST_DIR={}", dist_dir.display());
    tauri_build::build()
}
