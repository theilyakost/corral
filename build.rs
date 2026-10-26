use std::env;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=corral.manifest");
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let manifest = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("corral.manifest");
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", manifest.display());
    }
}
