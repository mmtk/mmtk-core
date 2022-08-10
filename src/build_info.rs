mod raw {
    // This is a full list of all the constants in built.rs from https://docs.rs/built/latest/built/index.html

    // /// The Continuous Integration platform detected during compilation.
    // pub const CI_PLATFORM: Option<&str> = None;
    // /// The full version.
    // pub const PKG_VERSION: &str = "0.1.0";
    // /// The major version.
    // pub const PKG_VERSION_MAJOR: &str = "0";
    // /// The minor version.
    // pub const PKG_VERSION_MINOR: &str = "1";
    // /// The patch version.
    // pub const PKG_VERSION_PATCH: &str = "0";
    // /// The pre-release version.
    // pub const PKG_VERSION_PRE: &str = "";
    // /// A colon-separated list of authors.
    // pub const PKG_AUTHORS: &str = "Lukas Lueg <lukas.lueg@gmail.com>";
    // /// The name of the package.
    // pub const PKG_NAME: &str = "example_project";
    // /// The description.
    // pub const PKG_DESCRIPTION: &str = "";
    // /// The homepage.
    // pub const PKG_HOMEPAGE: &str = "";
    // /// The license.
    // pub const PKG_LICENSE: &str = "MIT";
    // /// The source repository as advertised in Cargo.toml.
    // pub const PKG_REPOSITORY: &str = "";
    // /// The target triple that was being compiled for.
    // pub const TARGET: &str = "x86_64-unknown-linux-gnu";
    // /// The host triple of the rust compiler.
    // pub const HOST: &str = "x86_64-unknown-linux-gnu";
    // /// `release` for release builds, `debug` for other builds.
    // pub const PROFILE: &str = "debug";
    // /// The compiler that cargo resolved to use.
    // pub const RUSTC: &str = "rustc";
    // /// The documentation generator that cargo resolved to use.
    // pub const RUSTDOC: &str = "rustdoc";
    // /// Value of OPT_LEVEL for the profile used during compilation.
    // pub const OPT_LEVEL: &str = "0";
    // /// The parallelism that was specified during compilation.
    // pub const NUM_JOBS: u32 = 8;
    // /// Value of DEBUG for the profile used during compilation.
    // pub const DEBUG: bool = true;
    // /// The features that were enabled during compilation.
    // pub const FEATURES: [&str; 0] = [];
    // /// The features as a comma-separated string.
    // pub const FEATURES_STR: &str = "";
    // /// The output of `rustc -V`
    // pub const RUSTC_VERSION: &str = "rustc 1.43.1 (8d69840ab 2020-05-04)";
    // /// The output of `rustdoc -V`
    // pub const RUSTDOC_VERSION: &str = "rustdoc 1.43.1 (8d69840ab 2020-05-04)";
    // /// If the crate was compiled from within a git-repository, `GIT_VERSION` contains HEAD's tag. The short commit id is used if HEAD is not tagged.
    // pub const GIT_VERSION: Option<&str> = Some("0.4.1-10-gca2af4f");
    // /// If the repository had dirty/staged files.
    // pub const GIT_DIRTY: Option<bool> = Some(true);
    // /// If the crate was compiled from within a git-repository, `GIT_HEAD_REF` contains full name to the reference pointed to by HEAD (e.g.: `refs/heads/master`). If HEAD is detached or the branch name is not valid UTF-8 `None` will be stored.
    // pub const GIT_HEAD_REF: Option<&str> = Some("refs/heads/master");
    // /// If the crate was compiled from within a git-repository, `GIT_COMMIT_HASH` contains HEAD's full commit SHA-1 hash.
    // pub const GIT_COMMIT_HASH: Option<&str> = Some("ca2af4f11bb8f4f6421c4cccf428bf4862573daf");
    // /// An array of effective dependencies as documented by `Cargo.lock`.
    // pub const DEPENDENCIES: [(&str, &str); 37] = [("autocfg", "1.0.0"), ("bitflags", "1.2.1"), ("built", "0.4.1"), ("cargo-lock", "4.0.1"), ("cc", "1.0.54"), ("cfg-if", "0.1.10"), ("chrono", "0.4.11"), ("example_project", "0.1.0"), ("git2", "0.13.6"), ("idna", "0.2.0"), ("jobserver", "0.1.21"), ("libc", "0.2.71"), ("libgit2-sys", "0.12.6+1.0.0"), ("libz-sys", "1.0.25"), ("log", "0.4.8"), ("matches", "0.1.8"), ("num-integer", "0.1.42"), ("num-traits", "0.2.11"), ("percent-encoding", "2.1.0"), ("pkg-config", "0.3.17"), ("proc-macro2", "1.0.17"), ("quote", "1.0.6"), ("semver", "1.0.0"), ("serde", "1.0.110"), ("serde_derive", "1.0.110"), ("smallvec", "1.4.0"), ("syn", "1.0.25"), ("time", "0.1.43"), ("toml", "0.5.6"), ("unicode-bidi", "0.3.4"), ("unicode-normalization", "0.1.12"), ("unicode-xid", "0.2.0"), ("url", "2.1.1"), ("vcpkg", "0.2.8"), ("winapi", "0.3.8"), ("winapi-i686-pc-windows-gnu", "0.4.0"), ("winapi-x86_64-pc-windows-gnu", "0.4.0")];
    // /// The effective dependencies as a comma-separated string.
    // pub const DEPENDENCIES_STR: &str = "autocfg 1.0.0, bitflags 1.2.1, built 0.4.1, cargo-lock 4.0.1, cc 1.0.54, cfg-if 0.1.10, chrono 0.4.11, example_project 0.1.0, git2 0.13.6, idna 0.2.0, jobserver 0.1.21, libc 0.2.71, libgit2-sys 0.12.6+1.0.0, libz-sys 1.0.25, log 0.4.8, matches 0.1.8, num-integer 0.1.42, num-traits 0.2.11, percent-encoding 2.1.0, pkg-config 0.3.17, proc-macro2 1.0.17, quote 1.0.6, semver 1.0.0, serde 1.0.110, serde_derive 1.0.110, smallvec 1.4.0, syn 1.0.25, time 0.1.43, toml 0.5.6, unicode-bidi 0.3.4, unicode-normalization 0.1.12, unicode-xid 0.2.0, url 2.1.1, vcpkg 0.2.8, winapi 0.3.8, winapi-i686-pc-windows-gnu 0.4.0, winapi-x86_64-pc-windows-gnu 0.4.0";
    // /// The built-time in RFC2822, UTC
    // pub const BUILT_TIME_UTC: &str = "Wed, 27 May 2020 18:12:39 +0000";
    // /// The target architecture, given by `CARGO_CFG_TARGET_ARCH`.
    // pub const CFG_TARGET_ARCH: &str = "x86_64";
    // /// The endianness, given by `CARGO_CFG_TARGET_ENDIAN`.
    // pub const CFG_ENDIAN: &str = "little";
    // /// The toolchain-environment, given by `CARGO_CFG_TARGET_ENV`.
    // pub const CFG_ENV: &str = "gnu";
    // /// The OS-family, given by `CARGO_CFG_TARGET_FAMILY`.
    // pub const CFG_FAMILY: &str = "unix";
    // /// The operating system, given by `CARGO_CFG_TARGET_OS`.
    // pub const CFG_OS: &str = "linux";
    // /// The pointer width, given by `CARGO_CFG_TARGET_POINTER_WIDTH`.
    // pub const CFG_POINTER_WIDTH: &str = "64";

    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

/// MMTk crate version such as 0.14.0
pub const MMTK_PKG_VERSION: &'static str = raw::PKG_VERSION;

/// Comma separated features enabled for this build
pub const MMTK_FEATURES: &'static str = raw::FEATURES_STR;

lazy_static! {
    /// Git version such as a96e8f991c91a81df51e7975849441f52fdbcdcc, or a96e8f991c91a81df51e7975849441f52fdbcdcc-dirty, or unknown-git-version if MMTk
    /// is not built from a git repo.
    pub static ref MMTK_GIT_VERSION: &'static str = &MMTK_GIT_VERSION_STRING;

    // Owned string
    static ref MMTK_GIT_VERSION_STRING: String = if raw::GIT_COMMIT_HASH.is_some() {
        format!("{}{}", raw::GIT_COMMIT_HASH.unwrap(), if raw::GIT_DIRTY.unwrap() { "-dirty" } else { "" })
    } else {
        "unknown-git-version".to_string()
    };
}