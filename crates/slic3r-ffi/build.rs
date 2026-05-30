// build.rs — drive the cmake build of libslic3r_ffi.so and generate Rust
// bindings against its header.
//
// On `cargo build`:
//   1. Run cmake configure (idempotent; cmake itself caches).
//   2. Run cmake build for the slic3r_ffi target. libslic3r is rebuilt
//      transitively on first build (~15 min cold). Subsequent builds are
//      incremental — usually a few seconds.
//   3. Run bindgen against ffi/slic3r_ffi.h.
//   4. Emit link-search + link-lib + rpath so the produced examples and
//      downstream binaries find libslic3r_ffi.so.0 at runtime.
//   5. Emit `cargo:metadata=LIB_DIR=...` so downstream crates (src-tauri)
//      can read DEP_SLIC3R_FFI_LIB_DIR and set the rpath on the final
//      binary — rustc-link-arg does NOT propagate through the dependency
//      graph.
//
// The cmake build directory lives at <workspace>/build/slic3r-ffi/ (a stable
// location outside cargo's target/) so it survives `cargo clean`. Wipe it
// manually for a full FFI rebuild.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.join("..").join("..").canonicalize().unwrap();
    let cmake_build_dir = workspace_root.join("build").join("slic3r-ffi");
    // Default to RelWithDebInfo for local development (backtraces
    // through libslic3r are essential when a slice misbehaves —
    // see the PR-3-11 segfault investigation). CI overrides to
    // `Release` because RelWithDebInfo's debug-symbol output
    // pushed the GitHub Actions runner past its ~14 G disk ceiling
    // mid-build (run 26332210186 failed at `'No space left on
    // device'` in MeshBoolean.cpp.o).
    println!("cargo:rerun-if-env-changed=N3O_SLIC3R_FFI_CMAKE_CONFIG");
    let cmake_config =
        env::var("N3O_SLIC3R_FFI_CMAKE_CONFIG").unwrap_or_else(|_| "RelWithDebInfo".into());
    let cmake_config = cmake_config.as_str();

    // ---- Sanity check: deps must be built ----

    let deps_prefix =
        workspace_root.join("external/OrcaSlicer/deps/build/OrcaSlicer_dep/usr/local");
    if !deps_prefix.exists() {
        panic!(
            "OrcaSlicer's dependency tree is not built yet ({} missing).\n\
             Run `./scripts/build.sh deps` from the workspace root once \
             (~17 min) before `cargo build`.",
            deps_prefix.display()
        );
    }

    // ---- cmake configure (idempotent) ----

    std::fs::create_dir_all(&cmake_build_dir).expect("create cmake build dir");
    run(
        Command::new("cmake")
            .arg("-S")
            .arg(&manifest_dir)
            .arg("-B")
            .arg(&cmake_build_dir)
            .arg("-G")
            .arg("Ninja Multi-Config")
            .env("CMAKE_POLICY_VERSION_MINIMUM", "3.5"),
        "cmake configure",
    );

    // ---- cmake build ----

    run(
        Command::new("cmake")
            .arg("--build")
            .arg(&cmake_build_dir)
            .arg("--config")
            .arg(cmake_config)
            .arg("--target")
            .arg("slic3r_ffi"),
        "cmake build slic3r_ffi",
    );

    let lib_dir = cmake_build_dir.join(cmake_config);

    // ---- bindgen ----

    let header = manifest_dir.join("ffi").join("slic3r_ffi.h");
    println!("cargo:rerun-if-changed={}", header.display());
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("ffi/slic3r_ffi.cpp").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("ffi/nanosvg_impl.cpp").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("CMakeLists.txt").display()
    );

    let bindings = bindgen::Builder::default()
        .header(header.to_string_lossy())
        .allowlist_function("slic3r_.*")
        .allowlist_type("slic3r_.*")
        .allowlist_var("SLIC3R_.*")
        .prepend_enum_name(false)
        .derive_default(true)
        .generate()
        .expect("failed to generate bindings for slic3r_ffi.h");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("write bindings.rs");

    // ---- Link directives ----

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=slic3r_ffi");
    // rpath for this crate's own examples/tests. Won't propagate to
    // downstream binaries — they read DEP_SLIC3R_FFI_LIB_DIR (below).
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());

    // Surface the library path to downstream crates that have us as a dep.
    // With `links = "slic3r_ffi"` in Cargo.toml, every `cargo:KEY=value` line
    // is exposed in the consumer's build.rs env as DEP_SLIC3R_FFI_<KEY>.
    println!("cargo:LIB_DIR={}", lib_dir.display());
}

fn run(cmd: &mut Command, label: &str) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("{label} failed to spawn: {e}"));
    assert!(status.success(), "{label} failed (exit {status:?})");
}
