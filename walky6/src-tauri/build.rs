fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let public_dir = std::path::Path::new(&manifest_dir).join("..").join("public");
    println!("cargo:rustc-env=WALKY6_PUBLIC_DIR={}", public_dir.display());
    tauri_build::build()
}
