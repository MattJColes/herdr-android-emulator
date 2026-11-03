// Generate the emulator's gRPC client from the vendored proto, with a bundled protoc.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: build scripts are single-threaded; prost-build reads PROTOC from the environment.
    unsafe { std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path()?) };
    tonic_prost_build::configure()
        .build_server(false)
        .compile_protos(&["proto/emulator_controller.proto"], &["proto", protoc_bin_vendored::include_path()?.to_str().unwrap()])?;
    Ok(())
}
