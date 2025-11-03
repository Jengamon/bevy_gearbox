fn main() {
    // Declare our custom cfg so rustc's check-cfg knows about it
    println!("cargo:rustc-check-cfg=cfg(compiler_nightly)");

    let meta = rustc_version::version_meta().expect("failed to read rustc version metadata");
    if let rustc_version::Channel::Nightly = meta.channel {
        println!("cargo:rustc-cfg=compiler_nightly");
    }
}

