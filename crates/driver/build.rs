//! Bakes the toolchain's sysroot into the binary: an rpath so `librustc_driver` is found at
//! runtime without `DYLD_LIBRARY_PATH`/`LD_LIBRARY_PATH`, and the toolchain name so the front-end can make cargo
//! build the analysed workspace with the exact same compiler.

use std::env;
use std::path::Path;
use std::process::Command;

fn main() {
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let out = Command::new(rustc)
        .args(["--print", "sysroot"])
        .output()
        .expect("rustc --print sysroot runs");
    let sysroot = String::from_utf8(out.stdout)
        .expect("the sysroot path is UTF-8")
        .trim()
        .to_owned();
    println!("cargo:rustc-link-arg=-Wl,-rpath,{sysroot}/lib");
    let toolchain = Path::new(&sysroot)
        .file_name()
        .expect("the sysroot is a toolchain directory")
        .to_string_lossy()
        .into_owned();
    println!("cargo:rustc-env=TURBOFLEX_DRIVER_TOOLCHAIN={toolchain}");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=rust-toolchain.toml");
}
