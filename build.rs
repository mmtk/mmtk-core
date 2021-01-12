use std::process::Command;

fn main() {
    #[cfg(feature = "malloc_tcmalloc")]
    build_tcmalloc();
    #[cfg(feature = "malloc_hoard")]
    build_hoard();
}

fn build_tcmalloc() {
    let mut bazel = Command::new("bazel");
    bazel
        .current_dir("src/tcmalloc")
        .args(&["build", "tcmalloc"])
        .status().expect("TCMalloc failed to build :(");
    let mut cp = Command::new("cp");
    cp
        .current_dir("src/tcmalloc/bazel-bin/tcmalloc")
        .args(&["libtcmalloc.lo", "libtcmalloc.a"])
        .status().expect("failed to copy :(");
    println!("cargo:rustc-link-lib=static=tcmalloc");
    println!("cargo:rustc-link-lib=dylib=stdc++");
    // println!("cargo:rustc-link-lib=dylib=tcmalloc");
    // println!("cargo:rustc-link-lib=static=absl");
    println!("cargo:rustc-link-search=native=/home/paiger/mmtk-core/src/tcmalloc/bazel-bin/tcmalloc");
}

fn build_hoard() {
    let args: &[&str; 0] = &[];
    let mut make = Command::new("make");
    make
        .current_dir("Hoard-3.13/src")
        .args(args)
        .status().expect("Failed to make Hoard");
    println!("cargo:rustc-link-lib=dylib=hoard");
    println!("cargo:rustc-link-search=native=/home/paiger/mmtk-core/Hoard-3.13/src")
}