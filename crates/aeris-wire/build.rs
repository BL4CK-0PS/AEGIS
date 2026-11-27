//! Generates Rust types from `proto/aeris/v1/*.proto`.
//!
//! Uses `protox` (a pure-Rust protobuf compiler) so builds and CI do not
//! depend on a system `protoc` binary.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let proto_root = manifest.join("../../proto");
    let files = [
        proto_root.join("aeris/v1/event.proto"),
        proto_root.join("aeris/v1/command.proto"),
    ];
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
    }
    let fds = protox::compile(&files, [&proto_root])?;
    prost_build::Config::new().compile_fds(fds)?;
    Ok(())
}
