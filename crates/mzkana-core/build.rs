// Compile Mozc protocol buffers using prost-build and vendored protoc.
// Vendored commands.proto sources fetched from:
// https://github.com/google/mozc/tree/master/src/protocol
// Commit hash recorded in protocol/commands.proto header comment.

use std::env;
use std::path::Path;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let proto_dir = Path::new(&manifest_dir).join("protocol");

    // Use vendored protoc from protoc-bin-vendored
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("protoc binary not found");

    let mut config = prost_build::Config::new();
    config.protoc_executable(&protoc);

    // Compile protocol/commands.proto with manifest_dir as include path
    // so that "protocol/..." imports resolve correctly
    config
        .compile_protos(
            &[proto_dir.join("commands.proto").to_string_lossy().to_string()],
            &[manifest_dir],
        )
        .expect("Failed to compile protos");

    println!("cargo:rerun-if-changed={}", proto_dir.display());
}
