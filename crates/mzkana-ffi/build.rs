use std::path::PathBuf;

fn main() {
    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_header = PathBuf::from(&crate_dir).join("include").join("mzkana.h");

    let config = cbindgen::Config::from_file(format!("{crate_dir}/cbindgen.toml"))
        .expect("failed to load cbindgen.toml");

    cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
        .expect("cbindgen failed")
        .write_to_file(&out_header);

    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");
    // Set SONAME so the dynamic linker stores "libmzkana.so" (not the build-dir
    // absolute path) in the NEEDED entry of fcitx5-mzkana.so.
    println!("cargo:rustc-link-arg=-Wl,-soname,libmzkana.so");
}
