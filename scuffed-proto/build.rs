fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("proto/stream_info.proto")?;
    tonic_build::compile_protos("proto/stream_auth.proto")?;

    Ok(())
}
