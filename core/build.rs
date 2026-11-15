fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=../proto/cards.proto");
    let schema = protox::compile(["../proto/cards.proto"], ["../proto"])?;
    prost_build::Config::new().compile_fds(schema)?;
    Ok(())
}
