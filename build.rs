//! Build script for richspace XFCE4 panel plugin
//!
//! NOTE: Final plugin .so is created by GCC in justfile, not by rustc.
//! This is because rustc hides C symbols in cdylib output.

fn main() {
    println!("cargo:rerun-if-changed=plugin.c");
    println!("cargo:rerun-if-changed=plugin.h");
}
