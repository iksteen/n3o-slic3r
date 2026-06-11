// build.rs — drive the cmake build of the slic3r_ffi library and generate Rust
// bindings against its header.
//
// On `cargo build`:
//   1. Run cmake configure (idempotent; cmake itself caches).
//   2. Run cmake build for the slic3r_ffi target. libslic3r is rebuilt
//      transitively on first build (~15 min cold). Subsequent builds are
//      incremental — usually a few seconds.
//   3. Run bindgen against ffi/slic3r_ffi.h.
//   4. Emit link-search + link-lib + (Linux) rpath so the produced examples and
//      downstream binaries find the library at runtime.
//   5. Emit `cargo:metadata=LIB_DIR=...` so downstream crates (src-tauri)
//      can read DEP_SLIC3R_FFI_LIB_DIR and set the rpath on the final
//      binary — rustc-link-arg does NOT propagate through the dependency
//      graph.
//
// The cmake build directory lives at <workspace>/build/slic3r-ffi{,-win}/ (a
// stable location outside cargo's target/) so it survives `cargo clean`. Wipe it
// manually for a full FFI rebuild.
//
// Windows (cross from Linux): build with `cargo xwin build --target
// x86_64-pc-windows-msvc` and WINCROSS_PREFIX pointing at the cross-deps prefix
// (packaging/windows-cross/build-deps.sh). The cmake build is driven through the
// clang-cl toolchain in packaging/windows-cross/; the OrcaSlicer submodule is
// patched in place (idempotent). Output is slic3r_ffi.dll + its import lib.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.join("..").join("..").canonicalize().unwrap();
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let cmake_arch = cmake_target_arch(&target_os, &target_arch);
    let macos_deployment_target = macos_deployment_target();
    let windows = target_os == "windows";
    let macos = target_os == "macos";

    let cmake_build_dir = workspace_root
        .join("build")
        .join(if windows { "slic3r-ffi-win" } else { "slic3r-ffi" });

    // Default to RelWithDebInfo for local development (backtraces through
    // libslic3r are essential when a slice misbehaves). CI overrides to Release
    // because RelWithDebInfo's debug-symbol output pushed the GitHub Actions
    // runner past its disk ceiling mid-build. The cross build defaults to
    // Release (smaller, and the cross-deps were built /MD-Release).
    println!("cargo:rerun-if-env-changed=N3O_SLIC3R_FFI_CMAKE_CONFIG");
    let cmake_config = env::var("N3O_SLIC3R_FFI_CMAKE_CONFIG")
        .unwrap_or_else(|_| if windows { "Release".into() } else { "RelWithDebInfo".into() });
    let cmake_config = cmake_config.as_str();

    // ---- Sanity check: deps must be built ----

    if windows {
        let prefix = env::var("WINCROSS_PREFIX").unwrap_or_default();
        if prefix.is_empty() || !Path::new(&prefix).join("lib").exists() {
            panic!(
                "WINCROSS_PREFIX must point at the cross-deps prefix (with lib/).\n\
                 Run `packaging/windows-cross/build-deps.sh` once, then build with\n\
                 `WINCROSS_PREFIX=<prefix> cargo xwin build --target x86_64-pc-windows-msvc`."
            );
        }
    } else {
        let deps_prefix = orca_deps_prefix(&workspace_root, &target_os, &target_arch);
        if !deps_prefix.exists() {
            panic!(
                "OrcaSlicer's dependency tree is not built yet ({} missing).\n\
                 Run `./scripts/build.sh deps` from the workspace root once \
                 (~17 min) before `cargo build`.",
                deps_prefix.display()
            );
        }
    }

    // ---- cmake configure (idempotent) ----

    if macos {
        invalidate_stale_macos_cmake_cache(&cmake_build_dir, &macos_deployment_target);
    }
    std::fs::create_dir_all(&cmake_build_dir).expect("create cmake build dir");

    let mut configure = Command::new("cmake");
    configure
        .arg("-S")
        .arg(&manifest_dir)
        .arg("-B")
        .arg(&cmake_build_dir)
        .arg("-Wno-dev")
        .arg("-DCMAKE_POLICY_VERSION_MINIMUM=3.5")
        .arg("-DCMAKE_POLICY_DEFAULT_CMP0167=OLD")
        .arg("-DCMAKE_POLICY_DEFAULT_CMP0175=OLD")
        .env("CMAKE_POLICY_VERSION_MINIMUM", "3.5");

    if windows {
        // Cross: the clang-cl/LLD toolchain + the cross-deps prefix. Single-config
        // Ninja (the cross toolchain is validated with CMAKE_BUILD_TYPE=Release).
        let wc = workspace_root.join("packaging/windows-cross");
        let prefix = env::var("WINCROSS_PREFIX").unwrap();
        let xwin_dir = env::var("XWIN_DIR").unwrap_or_else(|_| {
            let cache = env::var("XDG_CACHE_HOME")
                .unwrap_or_else(|_| format!("{}/.cache", env::var("HOME").unwrap()));
            format!("{cache}/cargo-xwin/xwin")
        });
        apply_orca_patches(&workspace_root, &wc);
        configure
            .arg("-G")
            .arg("Ninja")
            .arg("-DCMAKE_BUILD_TYPE=Release")
            .arg(toolchain_arg("CMAKE_TOOLCHAIN_FILE", &wc.join("toolchain.cmake")))
            .arg(toolchain_arg("CMAKE_USER_MAKE_RULES_OVERRIDE", &wc.join("override.cmake")))
            .arg(toolchain_arg("CMAKE_PROJECT_INCLUDE", &wc.join("rc-sdk-includes.cmake")))
            .arg(format!("-DCMAKE_PREFIX_PATH={prefix}"))
            .arg("-DSLIC3R_GUI=OFF")
            .arg("-DSLIC3R_BUILD_SANDBOXES=OFF")
            .arg("-DBUILD_TESTS=OFF")
            .arg("-DORCA_TOOLS=OFF")
            .arg("-DORCA_BUILD_FFI=OFF")
            .arg("-DSLIC3R_STATIC=ON")
            .env("XWIN_DIR", &xwin_dir)
            .env("WINCROSS_PREFIX", &prefix);
    } else {
        configure.arg("-G").arg("Ninja Multi-Config");
        if macos {
            configure
                .arg(format!("-DCMAKE_OSX_ARCHITECTURES={cmake_arch}"))
                .arg(format!(
                    "-DCMAKE_OSX_DEPLOYMENT_TARGET={macos_deployment_target}"
                ))
                .arg(format!(
                    "-DCMAKE_PREFIX_PATH={}",
                    orca_deps_prefix(&workspace_root, &target_os, &target_arch).display()
                ));
        }
    }
    run(&mut configure, "cmake configure");

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

    // Single-config Ninja (windows) puts artifacts directly in the build dir;
    // Ninja Multi-Config (linux) puts them under a per-config subdir.
    let lib_dir = if windows {
        cmake_build_dir.clone()
    } else {
        cmake_build_dir.join(cmake_config)
    };

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

    // The header is ABI-clean (only <stddef.h>/<stdint.h>, fixed-width types), so
    // host-target bindgen produces correct bindings for the windows-msvc target.
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

    if windows {
        // Windows resolves DLLs from the executable's directory, not via rpath.
        // Drop the runtime DLLs next to this crate's examples/tests/bins so they
        // can run: the shim itself, plus the vendored GMP/MPFR (which OrcaSlicer
        // ships as DLLs, so slic3r_ffi.dll imports libgmp-10.dll at runtime).
        copy_runtime_dll(&lib_dir.join("slic3r_ffi.dll"), &out_dir);
        if let Ok(prefix) = env::var("WINCROSS_PREFIX") {
            let bin = Path::new(&prefix).join("bin");
            copy_runtime_dll(&bin.join("libgmp-10.dll"), &out_dir);
            copy_runtime_dll(&bin.join("libmpfr-4.dll"), &out_dir);
        }
    } else if macos {
        if let Some(dylib) = find_macos_runtime_dylib(&lib_dir) {
            copy_runtime_file(&dylib, &out_dir);
        }
        // rpath for this crate's own examples/tests plus the final Tauri app.
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    } else {
        // rpath for this crate's own examples/tests. Won't propagate to
        // downstream binaries — they read DEP_SLIC3R_FFI_LIB_DIR (below).
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
    }

    // Surface the library path to downstream crates that have us as a dep.
    // With `links = "slic3r_ffi"` in Cargo.toml, every `cargo:KEY=value` line
    // is exposed in the consumer's build.rs env as DEP_SLIC3R_FFI_<KEY>.
    println!("cargo:LIB_DIR={}", lib_dir.display());
}

// A -D<var>=<path> arg with the path as a normal (forward-slash) string — CMake
// accepts forward slashes everywhere, including on Windows-targeted builds.
fn toolchain_arg(var: &str, path: &Path) -> String {
    format!("-D{var}={}", path.display())
}

fn cmake_target_arch(target_os: &str, target_arch: &str) -> String {
    match (target_os, target_arch) {
        ("macos", "aarch64") => "arm64".to_string(),
        _ => target_arch.to_string(),
    }
}

fn macos_deployment_target() -> String {
    match env::var("MACOSX_DEPLOYMENT_TARGET") {
        Ok(value) if version_at_least(&value, "11.3") => value,
        _ => "11.3".to_string(),
    }
}

fn version_at_least(value: &str, minimum: &str) -> bool {
    fn parse(version: &str) -> Vec<u32> {
        version
            .split('.')
            .map(|part| part.parse::<u32>().unwrap_or(0))
            .collect()
    }

    let lhs = parse(value);
    let rhs = parse(minimum);
    let len = lhs.len().max(rhs.len());

    for idx in 0..len {
        let a = *lhs.get(idx).unwrap_or(&0);
        let b = *rhs.get(idx).unwrap_or(&0);
        if a != b {
            return a > b;
        }
    }

    true
}

fn invalidate_stale_macos_cmake_cache(build_dir: &Path, deployment_target: &str) {
    let cache_path = build_dir.join("CMakeCache.txt");
    let cache = match fs::read_to_string(&cache_path) {
        Ok(cache) => cache,
        Err(_) => return,
    };
    let wanted = format!("CMAKE_OSX_DEPLOYMENT_TARGET:STRING={deployment_target}");
    if cache.contains(&wanted) {
        return;
    }

    let _ = fs::remove_dir_all(build_dir);
}

fn orca_deps_prefix(workspace_root: &Path, target_os: &str, target_arch: &str) -> PathBuf {
    if let Ok(prefix) = env::var("N3O_SLIC3R_ORCA_DEPS_PREFIX") {
        return PathBuf::from(prefix);
    }

    match target_os {
        "macos" => {
            let orca_arch = match target_arch {
                "aarch64" => "arm64",
                other => other,
            };
            workspace_root
                .join("external/OrcaSlicer/deps/build")
                .join(orca_arch)
                .join("OrcaSlicer_dep/usr/local")
        }
        _ => workspace_root.join("external/OrcaSlicer/deps/build/OrcaSlicer_dep/usr/local"),
    }
}

// Apply packaging/windows-cross/patches/*.patch to the OrcaSlicer submodule in
// place (the submodule tree is otherwise left pinned). Idempotent: a patch that
// is already applied (reverse-apply --check succeeds) is skipped.
fn apply_orca_patches(workspace_root: &Path, wc: &Path) {
    let orca = workspace_root.join("external/OrcaSlicer");
    let patches_dir = wc.join("patches");
    let mut patches: Vec<PathBuf> = match std::fs::read_dir(&patches_dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map_or(false, |x| x == "patch"))
            .collect(),
        Err(_) => return,
    };
    patches.sort();
    for p in patches {
        let already = Command::new("git")
            .args(["-C"])
            .arg(&orca)
            .args(["apply", "--reverse", "--check"])
            .arg(&p)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if already {
            continue;
        }
        let ok = Command::new("git")
            .args(["-C"])
            .arg(&orca)
            .arg("apply")
            .arg(&p)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            println!(
                "cargo:warning=could not apply {} to the OrcaSlicer submodule",
                p.display()
            );
        }
    }
}

// Copy a runtime DLL next to the test/example/bin output dirs. OUT_DIR is
// target/<triple>/<profile>/build/<pkg-hash>/out — the profile dir (where
// examples/ and the bins live) is four ancestors up.
fn copy_runtime_dll(src: &Path, out_dir: &Path) {
    copy_runtime_file(src, out_dir);
}

fn copy_runtime_file(src: &Path, out_dir: &Path) {
    if !src.exists() {
        return;
    }
    let name = match src.file_name() {
        Some(n) => n,
        None => return,
    };
    if let Some(profile_dir) = out_dir.ancestors().nth(3) {
        for sub in ["", "examples", "deps"] {
            let dest_dir = profile_dir.join(sub);
            if std::fs::create_dir_all(&dest_dir).is_ok() {
                let _ = std::fs::copy(src, dest_dir.join(name));
            }
        }
    }
}

fn find_macos_runtime_dylib(lib_dir: &Path) -> Option<PathBuf> {
    let direct = lib_dir.join("libslic3r_ffi.dylib");
    if direct.exists() {
        return Some(direct);
    }

    let mut matches: Vec<PathBuf> = fs::read_dir(lib_dir)
        .ok()?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| {
                    name.starts_with("libslic3r_ffi")
                        && name.ends_with(".dylib")
                        && !path.is_symlink()
                })
                .unwrap_or(false)
        })
        .collect();
    matches.sort();
    matches.into_iter().next()
}

fn run(cmd: &mut Command, label: &str) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("{label} failed to spawn: {e}"));
    assert!(status.success(), "{label} failed (exit {status:?})");
}
