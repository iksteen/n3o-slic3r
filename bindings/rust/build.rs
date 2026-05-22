// build.rs — locate libslic3r_ffi and generate Rust bindings.
//
// Header location: SLIC3R_FFI_INCLUDE_DIR (env), else <repo>/ffi
// Library location: SLIC3R_FFI_LIB_DIR (env), else <repo>/build/ffi/RelWithDebInfo
//
// We set an rpath to the library directory so `cargo run --example` works
// without needing LD_LIBRARY_PATH. This is a dev-convenience; package builds
// should set RUSTFLAGS / install the .so to a system location instead.

use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo_root = manifest_dir.join("..").join("..").canonicalize().unwrap();

    let include_dir = env::var("SLIC3R_FFI_INCLUDE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_root.join("ffi"));

    let lib_dir = env::var("SLIC3R_FFI_LIB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_root.join("build").join("ffi").join("RelWithDebInfo"));

    let header = include_dir.join("slic3r_ffi.h");
    println!("cargo:rerun-if-changed={}", header.display());
    println!("cargo:rerun-if-env-changed=SLIC3R_FFI_INCLUDE_DIR");
    println!("cargo:rerun-if-env-changed=SLIC3R_FFI_LIB_DIR");

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=slic3r_ffi");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());

    let bindings = bindgen::Builder::default()
        .header(header.to_string_lossy())
        .allowlist_function("slic3r_.*")
        .allowlist_type("slic3r_.*")
        .allowlist_var("SLIC3R_.*")
        .prepend_enum_name(false)
        .derive_default(true)
        .generate()
        .expect("failed to generate bindings for slic3r_ffi.h");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("bindings.rs");
    bindings.write_to_file(out).expect("write bindings.rs");
}
