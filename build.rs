fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/homeroom.proto");
    tonic_prost_build::compile_protos("proto/homeroom.proto")?;
    println!("cargo:rerun-if-changed=proto/classwork_evidence.proto");
    println!("cargo:rerun-if-changed=proto/mutation_evidence.proto");
    if std::env::var_os("CARGO_FEATURE_DESKTOP").is_some() {
        tonic_prost_build::configure()
            .build_client(false)
            .build_server(false)
            .compile_protos(&["proto/classwork_evidence.proto", "proto/mutation_evidence.proto"], &["proto"])?;
    }
    Ok(())
}
