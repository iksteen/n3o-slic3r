// src-tauri/build.rs
//
// In addition to running Tauri's own build step, set an rpath on the produced
// binary so it can find libslic3r_ffi.so.0 at runtime.
//
// Why this lives here (and not in slic3r-ffi's build.rs): Cargo's
// `rustc-link-arg` build-script instruction only applies to binaries/cdylibs
// produced by the build script's OWN package — it does NOT propagate to
// downstream consumers. The library's build.rs gets `rustc-link-search` and
// `rustc-link-lib` propagated (so linking succeeds), but rpath does not. So
// the consumer that produces the binary has to set its own rpath.

use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    // Default: <repo>/build/ffi/RelWithDebInfo (cmake output for the vendored
    // slic3r_ffi target).
    let lib_dir = env::var("SLIC3R_FFI_LIB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            manifest_dir
                .join("..")
                .join("build")
                .join("ffi")
                .join("RelWithDebInfo")
        });
    let lib_dir = lib_dir.canonicalize().unwrap_or(lib_dir);

    println!("cargo:rerun-if-env-changed=SLIC3R_FFI_LIB_DIR");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());

    tauri_build::build()
}
