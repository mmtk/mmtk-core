// build.rs

use std::process::Command;
use std::env;
use std::path::Path;

fn main() {
    cc::Build::new().cpp(true).file("perfmon.cpp").flag("-std=c++14").compile("perfmon");
    println!("cargo:rerun-if-changed=perfmon.cpp");
    println!("cargo:rustc-flags=-lpfm");
}