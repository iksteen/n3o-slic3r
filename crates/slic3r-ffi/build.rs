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
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.join("..").join("..").canonicalize().unwrap();
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let windows = target_os == "windows";
    let macos = target_os == "macos";
    // Cross-compiling a macOS target from a non-macOS host (Linux + osxcross):
    // build.rs itself is compiled for the build host, so cfg!(target_os) is the
    // HOST os. A macOS *target* on a non-macOS host means we must drive cmake
    // through the osxcross toolchain rather than the host clang. On a native
    // macOS host this is false and the build is unchanged.
    let macos_cross = macos && !cfg!(target_os = "macos");

    // OrcaSlicer's macOS build namespaces everything by arch — the deps
    // install prefix (build_release_macos.sh) and our own cmake build dir —
    // so an arm64 and an x86_64 tree coexist for cross-compiling / universal
    // builds. Cargo's TARGET_ARCH is the LLVM spelling (aarch64); OrcaSlicer
    // and Apple tooling use the uname spelling (arm64). For x86_64 they agree.
    let mac_arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") | Err(_) => "arm64".to_string(),
        Ok(other) => other.to_string(),
    };
    let mac_deps_prefix = workspace_root.join(format!(
        "external/OrcaSlicer/deps/build/{mac_arch}/OrcaSlicer_dep/usr/local"
    ));

    let cmake_build_dir = workspace_root.join("build").join(if windows {
        "slic3r-ffi-win".to_string()
    } else if macos {
        // Per-arch so a native arm64 build and a cross x86_64 build each keep
        // their own cmake cache (no reconfigure churn when switching targets).
        format!("slic3r-ffi-{mac_arch}")
    } else {
        "slic3r-ffi".to_string()
    });

    // Follow the cargo profile: a **release** build gets an optimized engine
    // (Release, -O3); a **debug** build keeps RelWithDebInfo for libslic3r
    // backtraces when a slice misbehaves. This matters most on macOS, where
    // OrcaSlicer deliberately strips RelWithDebInfo to -O0 under Clang (a
    // GUI-debugging choice — see its CMakeLists ~L542); a release build must
    // not inherit that, or slicing crawls. `N3O_SLIC3R_FFI_CMAKE_CONFIG`
    // overrides either way (CI forces Release; set RelWithDebInfo to debug the
    // engine itself in an otherwise-release build). Windows cross is always
    // Release. The bundle configs (tauri.macos.conf.json, the macos-cross
    // bundle script) embed the `Release/` subdir — `tauri build` is always a
    // release build, so the dylib lands there.
    println!("cargo:rerun-if-env-changed=N3O_SLIC3R_FFI_CMAKE_CONFIG");
    let is_release = env::var("PROFILE").as_deref() == Ok("release");
    let cmake_config = env::var("N3O_SLIC3R_FFI_CMAKE_CONFIG").unwrap_or_else(|_| {
        if windows || is_release {
            "Release".into()
        } else {
            "RelWithDebInfo".into()
        }
    });
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
        let deps_prefix = if macos {
            mac_deps_prefix.clone()
        } else {
            workspace_root.join("external/OrcaSlicer/deps/build/OrcaSlicer_dep/usr/local")
        };
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

    std::fs::create_dir_all(&cmake_build_dir).expect("create cmake build dir");

    let mut configure = Command::new("cmake");
    configure
        .arg("-S")
        .arg(&manifest_dir)
        .arg("-B")
        .arg(&cmake_build_dir)
        .env("CMAKE_POLICY_VERSION_MINIMUM", "3.5");

    // Embed the pinned OrcaSlicer submodule SHA in the shim's version string so
    // a built binary reports exactly which engine revision it links. Best-effort
    // — outside a git checkout (e.g. a source tarball) the command fails and the
    // SHA is simply omitted rather than breaking the build.
    if let Some(sha) = Command::new("git")
        .arg("-C")
        .arg(workspace_root.join("external/OrcaSlicer"))
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
    {
        configure.arg(format!("-DN3O_ORCA_SHA={sha}"));
    }

    // Wave-overhang engine feature — carried as build-time patches on the
    // pinned OrcaSlicer submodule (crates/slic3r-ffi/patches/wave-overhangs/).
    // Apply on EVERY target, before cmake configure, so a clean build links
    // the wave module that the scraped option tables already advertise.
    // Idempotent (reverse-apply --check skips an already-patched tree).
    apply_submodule_patches(&workspace_root, &manifest_dir.join("patches/wave-overhangs"));

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
        apply_submodule_patches(&workspace_root, &wc);
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
                // The CMakeLists default deps prefix is the Linux layout
                // (deps/build/OrcaSlicer_dep/...). On macOS the prefix is
                // arch-namespaced, so point CMAKE_PREFIX_PATH at it explicitly.
                .arg(format!("-DCMAKE_PREFIX_PATH={}", mac_deps_prefix.display()))
                // Build libslic3r + the shim for the cargo target arch (native
                // arm64 or cross x86_64), not the host default — otherwise a
                // cross build would link x86_64 cargo objects against an arm64
                // dylib. Deployment target matches the deps (built at 11.3).
                .arg(format!("-DCMAKE_OSX_ARCHITECTURES={mac_arch}"))
                .arg("-DCMAKE_OSX_DEPLOYMENT_TARGET=11.3");
            if macos_cross {
                // Cross from Linux: drive the build through osxcross. The wrapper
                // (packaging/macos-cross/build.sh) exports the OSXCROSS_* env the
                // toolchain file reads (it picks the arch via OSXCROSS_HOST) and
                // MACCROSS_PREFIX. Fail early with guidance if it's not set up.
                // OrcaSlicer builds a host dev-tool (encoding-check) and runs it
                // during the build to verify source encodings. Cross-built it's a
                // Mach-O binary that can't execute on the Linux build host
                // (Exec format error). OrcaSlicer auto-disables it under
                // IS_CROSS_COMPILE, but that isn't tripped by the osxcross setup —
                // disable it explicitly. The check is irrelevant to the shim.
                configure.arg("-DSLIC3R_ENC_CHECK=OFF");
                // Gate the __isPlatformVersionAtLeast shim to this exact host/
                // target combo (see ffi/macos_availability_shim.mm).
                configure.arg("-DN3O_MACOS_CROSS=ON");
                let tc = workspace_root.join("packaging/macos-cross/toolchain.cmake");
                if env::var_os("OSXCROSS_TARGET_DIR").is_none() {
                    panic!(
                        "Cross-building a macOS target from this host needs osxcross.\n\
                         Build through `packaging/macos-cross/build.sh <arch> <cargo|tauri ...>`,\n\
                         which exports OSXCROSS_TARGET_DIR/OSXCROSS_HOST/MACCROSS_PREFIX, or\n\
                         export them yourself (see packaging/macos-cross/README.md)."
                    );
                }
                configure.arg(toolchain_arg("CMAKE_TOOLCHAIN_FILE", &tc));
            }
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

    // macOS: keep a stable `build/slic3r-ffi-current` symlink pointing at the
    // arch-specific build dir we just produced. tauri.macos.conf.json embeds
    // `build/slic3r-ffi-current/Release/libslic3r_ffi.0.dylib` (bundling is
    // always a release build, so the dylib is in the Release subdir), so this
    // lets a native `tauri build` and a cross `tauri build --target
    // x86_64-apple-darwin` each bundle the matching-arch dylib through one
    // static config path — the cargo build (which runs this script for the
    // target arch) always repoints the link just before tauri bundles.
    if macos {
        let link = workspace_root.join("build").join("slic3r-ffi-current");
        if let Err(e) = update_symlink(&link, &format!("slic3r-ffi-{mac_arch}")) {
            println!("cargo:warning=could not update slic3r-ffi-current symlink: {e}");
        }
    }

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
    // Re-run (and re-apply) if a carried wave-overhang patch changes.
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("patches/wave-overhangs/patches").display()
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

// Replace `link` with a relative symlink to `target` (a sibling name). Used for
// the macOS `slic3r-ffi-current` pointer. Unix-only — the only macOS-targeting
// build host is a Mac; the Windows target cross-builds from Linux (also unix).
#[cfg(unix)]
fn update_symlink(link: &Path, target: &str) -> std::io::Result<()> {
    let _ = std::fs::remove_file(link);
    std::os::unix::fs::symlink(target, link)
}
#[cfg(not(unix))]
fn update_symlink(_link: &Path, _target: &str) -> std::io::Result<()> {
    Ok(())
}

// Apply `<base>/patches/*.patch` to the OrcaSlicer submodule in place (the
// submodule tree is otherwise left pinned). Used for both the windows-cross
// build patches and the wave-overhang engine carry. Idempotent: a patch that
// is already applied (reverse-apply --check succeeds) is skipped, so it's safe
// to run on every build and on an already-patched tree.
fn apply_submodule_patches(workspace_root: &Path, base: &Path) {
    let orca = workspace_root.join("external/OrcaSlicer");
    let patches_dir = base.join("patches");
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
            // The reverse-check above already proved the patch isn't applied,
            // so a failed forward-apply is unambiguous — fail the build rather
            // than silently shipping an engine missing the wave-overhang (or
            // cross-build) carry. Check the submodule is at its pinned commit.
            panic!(
                "failed to apply {} to the OrcaSlicer submodule (tree not at the \
                 pinned commit, or the patch no longer applies)",
                p.display()
            );
        }
    }
}

// Copy a runtime DLL next to the test/example/bin output dirs. OUT_DIR is
// target/<triple>/<profile>/build/<pkg-hash>/out — the profile dir (where
// examples/ and the bins live) is four ancestors up.
fn copy_runtime_dll(src: &Path, out_dir: &Path) {
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

fn run(cmd: &mut Command, label: &str) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("{label} failed to spawn: {e}"));
    assert!(status.success(), "{label} failed (exit {status:?})");
}
