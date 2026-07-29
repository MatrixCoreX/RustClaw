fn main() {
    println!("cargo:rerun-if-env-changed=TARGET");
    let target = std::env::var("TARGET").expect("Cargo provides TARGET to build scripts");
    println!("cargo:rustc-env=RUSTCLAW_BUILD_TARGET={target}");
}
