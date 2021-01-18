// use std::process::Command;

// fn main() {
//     #[cfg(feature = "malloc_tcmalloc")]
//     build_tcmalloc();
// }

fn main() {
    
}

// fn build_tcmalloc() {
//     let mut bazel = Command::new("bazel");
//     bazel
//         .current_dir("src/tcmalloc")
//         .args(&["build", "tcmalloc"])
//         .status().expect("TCMalloc failed to build :(");
//     let mut cp = Command::new("cp");
//     cp
//         .current_dir("src/tcmalloc/bazel-bin/tcmalloc")
//         .args(&["libtcmalloc.lo", "libtcmalloc.a"])
//         .status().expect("failed to copy :(");
//     println!("cargo:rustc-link-lib=static=tcmalloc");
//     println!("cargo:rustc-link-lib=dylib=stdc++");
//     // println!("cargo:rustc-link-lib=dylib=tcmalloc");
//     // println!("cargo:rustc-link-lib=static=absl");
//     println!("cargo:rustc-link-search=native=/home/paiger/mmtk-core/src/tcmalloc/bazel-bin/tcmalloc");
//     // println!("cargo:rustc-link-search=dependency=/home/paiger/mmtk-core/src/tcmalloc/bazel-bin/tcmalloc");
// }