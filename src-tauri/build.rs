// src-tauri/build.rs
//
// On Unix, set an rpath on the produced binary so it can find libslic3r_ffi.so.0
// at runtime. Cargo's `rustc-link-arg` does NOT propagate through the dependency
// graph, so the binary-producing crate has to emit it itself. We pick up the
// library path from the slic3r-ffi crate's build-script metadata: that crate
// declares `links = "slic3r_ffi"` and emits `cargo:metadata=LIB_DIR=...`,
// which Cargo surfaces as DEP_SLIC3R_FFI_LIB_DIR in our environment here.
//
// On Windows there is no rpath: the loader resolves slic3r_ffi.dll from the
// executable's own directory, and the slic3r-ffi build script copies the DLL
// there. So the rpath link-arg is Unix-only (it is also a GNU-ld syntax that
// lld-link does not understand).

use std::env;

fn main() {
    let lib_dir = env::var("DEP_SLIC3R_FFI_LIB_DIR")
        .expect("DEP_SLIC3R_FFI_LIB_DIR not set — is slic3r-ffi a dependency?");

    println!("cargo:rerun-if-env-changed=DEP_SLIC3R_FFI_LIB_DIR");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{lib_dir}");
    }

    tauri_build::build()
}
