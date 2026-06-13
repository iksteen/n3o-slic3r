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
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        // rpath at the cmake build dir: lets the freshly-built binary (and
        // `tauri dev`) find libslic3r_ffi.<ver>.{dylib,so} straight out of the
        // build tree, with no install step.
        println!("cargo:rustc-link-arg=-Wl,-rpath,{lib_dir}");
    }
    if target_os == "macos" {
        // Second rpath for the bundled .app: `tauri build` copies the dylib
        // into Contents/Frameworks (see tauri.macos.conf.json) and the binary
        // lives in Contents/MacOS, so Frameworks is one level up. With this the
        // .app is relocatable — the absolute build-dir rpath above simply fails
        // to resolve on another machine, and dyld falls through to this one.
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    }

    tauri_build::build()
}
