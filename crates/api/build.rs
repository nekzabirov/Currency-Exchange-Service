fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Rebuild only when the contract itself changes: otherwise every edit to any file would drag
    // protoc along.
    println!("cargo:rerun-if-changed=../../proto/currency/v1/currency.proto");

    tonic_prost_build::configure()
        // Server only: we never call ourselves, and clients generate their own stubs from the
        // `.proto` file.
        .build_client(false)
        .compile_protos(
            &["../../proto/currency/v1/currency.proto"],
            &["../../proto"],
        )?;

    Ok(())
}
