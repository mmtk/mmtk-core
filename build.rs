fn main() {
    cc::Build::new().cpp(true).file("perfevent.cpp").flag("-std=c++14").compile("perfevent");
    println!("cargo:rerun-if-changed=perfevent.cpp");
    println!("cargo:rustc-flags=-lpfm");
}