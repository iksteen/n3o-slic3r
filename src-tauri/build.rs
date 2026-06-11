// src-tauri/build.rs
//
// On Unix, set an rpath on the produced binary so it can find the slic3r_ffi
// shared library at runtime. Cargo's `rustc-link-arg` does NOT propagate through
// the dependency graph, so the binary-producing crate has to emit it itself. We
// pick up the library path from the slic3r-ffi crate's build-script metadata:
// that crate declares `links = "slic3r_ffi"` and emits `cargo:metadata=LIB_DIR=...`,
// which Cargo surfaces as DEP_SLIC3R_FFI_LIB_DIR in our environment here.
//
// On macOS, the packaged app bundles libslic3r_ffi.dylib into
// `Contents/Frameworks`, so the runtime rpath must include that location in
// addition to the build-tree path used by dev runs.
//
// On Windows there is no rpath: the loader resolves slic3r_ffi.dll from the
// executable's own directory, and the slic3r-ffi build script copies the DLL
// there. So the rpath link-arg is Unix-only (it is also a GNU-ld syntax that
// lld-link does not understand).

use std::env;

fn main() {
    let lib_dir = env::var("DEP_SLIC3R_FFI_LIB_DIR")
        .expect("DEP_SLIC3R_FFI_LIB_DIR not set — is slic3r-ffi a dependency?");
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    println!("cargo:rerun-if-env-changed=DEP_SLIC3R_FFI_LIB_DIR");
    if target_os != "windows" {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{lib_dir}");
        if target_os == "macos" {
            println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
        }
    }

    tauri_build::build()
}
