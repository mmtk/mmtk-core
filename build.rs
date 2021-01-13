use std::process::Command;

fn main() {
    #[cfg(feature = "malloc_tcmalloc")]
    build_tcmalloc();
    #[cfg(feature = "malloc_scalloc")]
    build_scalloc();
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
    // println!("cargo:rustc-link-search=dependency=/home/paiger/mmtk-core/src/tcmalloc/bazel-bin/tcmalloc");
}

fn build_scalloc() {
    let mut gyp = Command::new("gyp");
    gyp
        .current_dir("scalloc")
        .args(&["build/gyp/gyp", "--depth=.", "scalloc.gyp"])
        .status().expect("Failed to generate scalloc build environment.");
    println!("cargo:rustc-env=BUILDTYPE=Release");
    let args: &[&str; 0] = &[];
    let mut make = Command::new("make");
    make
    .current_dir("scalloc")
    .args(args)
    .status().expect("Failed to build scalloc.");
    println!("cargo:rustc-link-lib=dylib=scalloc");
    println!("cargo:rustc-link-search=native=/home/paiger/mmtk-core/scalloc/out/Release/lib.target")
}