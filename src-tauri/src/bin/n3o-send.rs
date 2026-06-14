//! `n3o-send` — a small headless CLI that pushes a *pre-sliced* file to a
//! configured printer instance, reusing the app's driver layer.
//!
//! Two paths, dispatched by the instance's stored connection kind:
//!   - **Bambu**: a pre-built `.gcode.3mf` (or `.platecycler.3mf`) bundle is
//!     FTPS-uploaded and a `project_file` print command published over MQTT.
//!     Unlike the in-app send, the bundle is uploaded *verbatim* — it's
//!     already sliced, so there's no raw-gcode → 3mf wrapping step. AMS
//!     routing defaults to the external spool; `--ams` maps each filament
//!     in the print (`T0,T1,…`) to an AMS slot on the fly.
//!   - **Snapmaker U1**: a raw `.gcode` is POSTed to Moonraker with
//!     `print=true`.
//!
//! Instances come from the same on-disk library the GUI uses
//! (`$XDG_CONFIG_HOME/n3o-slic3r/printers/<id>.toml`); no Tauri runtime is
//! involved. The path resolution mirrors the GUI's Linux target — pass
//! `--printers-dir` (or set `N3O_PRINTERS_DIR`) to point elsewhere.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Instant;

use n3o_slic3r_lib::core::driver::bambu::connection::{BambuConfig, BambuDriver};
use n3o_slic3r_lib::core::driver::snapmaker::{U1Config, U1Driver};
use n3o_slic3r_lib::core::driver::traits::AmsMappingV2;
use n3o_slic3r_lib::core::driver::{
    ConnectionState, Driver, DriverId, JobState, PrinterStatus, SendPayload,
};
use n3o_slic3r_lib::core::printer::{instance_storage, ConnectionInfo, PrinterInstance};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    init_tracing();
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("n3o-send: {msg}\n");
            print_usage();
            return ExitCode::from(2);
        }
    };
    if args.help {
        print_usage();
        return ExitCode::SUCCESS;
    }
    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("n3o-send: error: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> Result<(), String> {
    let dir = resolve_printers_dir(args.printers_dir.clone())?;
    let instances = instance_storage::load_from_disk(&dir)
        .map_err(|e| format!("loading printer library from {}: {e}", dir.display()))?;

    if args.list {
        list_instances(&instances, &dir);
        return Ok(());
    }

    let id = args
        .instance_id
        .as_deref()
        .ok_or("missing <instance-id> (run with --list to see configured printers)")?;
    let file = args.file.as_deref().ok_or("missing <file>")?;

    let inst = instances.iter().find(|i| i.id == id).ok_or_else(|| {
        format!(
            "no printer instance `{id}` in {} (try --list)",
            dir.display()
        )
    })?;
    let conn = inst.connection.as_ref().ok_or_else(|| {
        format!(
            "instance `{id}` ({}) has no connection configured — set it in the app first",
            inst.display_name
        )
    })?;

    let bytes = std::fs::read(file).map_err(|e| format!("reading {}: {e}", file.display()))?;

    match conn {
        ConnectionInfo::Bambu { host, access_code } => {
            send_bambu(inst, host, access_code, file, bytes, &args).await
        }
        ConnectionInfo::U1 { host, port } => send_u1(inst, host, *port, file, bytes, &args).await,
    }
}

/// Bambu: FTPS-upload the `.gcode.3mf` bundle verbatim, publish the
/// `project_file` print command, then keep the connection alive long
/// enough for the (fire-and-forget) MQTT publish to flush and the printer
/// to confirm the job started.
async fn send_bambu(
    inst: &PrinterInstance,
    host: &str,
    access_code: &str,
    file: &Path,
    bytes: Vec<u8>,
    args: &Args,
) -> Result<(), String> {
    if !has_ext(file, "3mf") {
        return Err(format!(
            "`{}` is not a .gcode.3mf / .platecycler.3mf bundle; the Bambu instance `{}` needs a \
             pre-sliced 3MF (raw .gcode is Snapmaker-only)",
            file.display(),
            inst.id
        ));
    }

    // Resolve AMS routing before touching the network so a bad spec fails
    // fast. No `--ams` ⇒ empty arrays ⇒ firmware uses the external spool.
    let (use_ams, ams_mapping, ams_mapping2) = match &args.ams {
        Some(spec) => parse_ams_spec(spec)?,
        None => (false, Vec::new(), Vec::new()),
    };

    eprintln!(
        "→ {} ({host}): uploading {} ({} KiB) to plate {}…",
        inst.display_name,
        file_name_of(file),
        bytes.len() / 1024,
        args.plate
    );
    if args.ams.is_some() {
        eprintln!(
            "  · AMS routing: {}",
            render_ams(&ams_mapping, &ams_mapping2)
        );
    }

    let mut driver = BambuDriver::new(
        DriverId(0),
        BambuConfig {
            host: host.to_owned(),
            access_code: access_code.to_owned(),
        },
    );
    // Subscribe before connecting so we observe the full Connecting→Connected
    // transition while waiting for the start confirmation.
    let status_rx = driver.subscribe_status();
    driver
        .connect()
        .await
        .map_err(|e| format!("connecting to Bambu at {host}: {e}"))?;

    let payload = SendPayload::Gcode3mf {
        bytes,
        plate_id: args.plate,
        use_ams,
        ams_mapping,
        ams_mapping2,
    };
    // CLI: no progress UI (it prints its own "✓ uploaded" line), so a no-op.
    let handle = driver
        .send(payload, std::sync::Arc::new(|_, _| {}))
        .await
        .map_err(|e| format!("send failed: {e}"))?;
    eprintln!(
        "✓ uploaded as {} (seq {}); waiting up to {}s for the printer to start…",
        handle.file_name, handle.id, args.wait_secs
    );

    watch_for_start(status_rx, args.wait_secs).await;

    // Graceful disconnect drains the rumqttc queue (the publish is ahead of
    // the disconnect frame), so this also guarantees the command flushed.
    let _ = driver.disconnect().await;
    Ok(())
}

/// Snapmaker U1: a single multipart POST to Moonraker that uploads the raw
/// G-code and starts the print (`print=true`). The HTTP call is fully
/// awaited, so a successful return means the print is underway — no
/// connection lifecycle or status wait needed.
async fn send_u1(
    inst: &PrinterInstance,
    host: &str,
    port: u16,
    file: &Path,
    bytes: Vec<u8>,
    args: &Args,
) -> Result<(), String> {
    if !has_ext(file, "gcode") {
        return Err(format!(
            "`{}` is not a raw .gcode; the Snapmaker instance `{}` needs plain G-code (the \
             .gcode.3mf / .platecycler.3mf bundle is Bambu-only)",
            file.display(),
            inst.id
        ));
    }
    if args.ams.is_some() {
        return Err(
            "--ams is Bambu-only; the U1's per-toolhead feed is fixed by the sliced file"
                .to_owned(),
        );
    }

    let mut file_name = args
        .name
        .clone()
        .unwrap_or_else(|| file_name_of(file).to_owned());
    if !file_name.to_ascii_lowercase().ends_with(".gcode") {
        file_name.push_str(".gcode");
    }

    eprintln!(
        "→ {} ({host}:{port}): uploading {} ({} KiB) and starting print…",
        inst.display_name,
        file_name,
        bytes.len() / 1024
    );

    let mut driver = U1Driver::new(
        DriverId(0),
        U1Config {
            host: host.to_owned(),
            port,
        },
    );
    let handle = driver
        .send(SendPayload::Gcode { bytes, file_name }, std::sync::Arc::new(|_, _| {}))
        .await
        .map_err(|e| format!("send failed: {e}"))?;
    eprintln!("✓ uploaded as {} and print started", handle.file_name);
    Ok(())
}

/// Watch the driver's status channel until the job reaches an active state
/// (Preparing/Printing), reports failure, or `wait_secs` elapses. Surfaces
/// connection issues (auth/network) as they appear so an unreachable printer
/// doesn't just time out silently.
async fn watch_for_start(mut rx: watch::Receiver<PrinterStatus>, wait_secs: u64) {
    let deadline = Instant::now() + Duration::from_secs(wait_secs);
    let mut last_conn = String::new();
    loop {
        {
            let status = rx.borrow_and_update();
            let conn = format!("{:?}", status.connection);
            if conn != last_conn {
                match &status.connection {
                    ConnectionState::Connected => eprintln!("  · connected to printer"),
                    ConnectionState::Reconnecting { reason, .. }
                    | ConnectionState::Disconnected { reason } => {
                        eprintln!("  · connection: {reason}")
                    }
                    ConnectionState::Connecting => {}
                }
                last_conn = conn;
            }
            if let Some(job) = &status.job {
                match &job.state {
                    JobState::Preparing | JobState::Printing => {
                        eprintln!("✓ print started ({:?})", job.state);
                        return;
                    }
                    JobState::Failed(reason) => {
                        eprintln!("✗ printer reported failure: {reason}");
                        return;
                    }
                    _ => {}
                }
            }
        }
        match tokio::time::timeout_at(deadline, rx.changed()).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => return, // driver dropped the sender
            Err(_) => {
                eprintln!(
                    "… no start confirmation within {wait_secs}s — the command was sent; \
                     check the printer (or raise --wait)."
                );
                return;
            }
        }
    }
}

fn list_instances(instances: &[PrinterInstance], dir: &Path) {
    if instances.is_empty() {
        println!("No printer instances in {}", dir.display());
        return;
    }
    println!("Printer instances in {}:", dir.display());
    for i in instances {
        let conn = match &i.connection {
            Some(ConnectionInfo::Bambu { host, .. }) => format!("bambu @ {host}"),
            Some(ConnectionInfo::U1 { host, port }) => format!("u1 @ {host}:{port}"),
            None => "(no connection)".to_owned(),
        };
        println!("  {:<18} {:<26} {}", i.id, i.display_name, conn);
    }
}

// ── argument parsing ──────────────────────────────────────────────────

struct Args {
    instance_id: Option<String>,
    file: Option<PathBuf>,
    plate: u32,
    name: Option<String>,
    ams: Option<String>,
    printers_dir: Option<PathBuf>,
    wait_secs: u64,
    list: bool,
    help: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            instance_id: None,
            file: None,
            plate: 1,
            name: None,
            ams: None,
            printers_dir: None,
            wait_secs: 15,
            list: false,
            help: false,
        }
    }
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = std::env::args().skip(1);
    let mut positionals = Vec::new();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                a.help = true;
                return Ok(a);
            }
            "--list" => a.list = true,
            "--plate" => {
                a.plate = take_val(&mut it, "--plate")?
                    .parse()
                    .map_err(|_| "--plate needs a non-negative integer".to_owned())?;
            }
            "--name" => a.name = Some(take_val(&mut it, "--name")?),
            "--ams" => a.ams = Some(take_val(&mut it, "--ams")?),
            "--printers-dir" => {
                a.printers_dir = Some(PathBuf::from(take_val(&mut it, "--printers-dir")?))
            }
            "--wait" => {
                a.wait_secs = take_val(&mut it, "--wait")?
                    .parse()
                    .map_err(|_| "--wait needs a number of seconds".to_owned())?;
            }
            s if s.starts_with("--") => return Err(format!("unknown option `{s}`")),
            _ => positionals.push(arg),
        }
    }
    let mut p = positionals.into_iter();
    a.instance_id = p.next();
    a.file = p.next().map(PathBuf::from);
    if let Some(extra) = p.next() {
        return Err(format!("unexpected extra argument `{extra}`"));
    }
    // Positionals are required unless we're just listing or showing help.
    if !a.list && !a.help {
        if a.instance_id.is_none() {
            return Err(
                "missing <instance-id> (run with --list to see configured printers)".to_owned(),
            );
        }
        if a.file.is_none() {
            return Err("missing <file> to send".to_owned());
        }
    }
    Ok(a)
}

fn take_val(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    it.next().ok_or_else(|| format!("{flag} needs a value"))
}

fn print_usage() {
    eprintln!(
        "n3o-send — send a pre-sliced file to a configured printer\n\
         \n\
         USAGE:\n\
         \x20   n3o-send <instance-id> <file> [options]\n\
         \x20   n3o-send --list\n\
         \n\
         ARGS:\n\
         \x20   <instance-id>   id of a printer instance (see --list)\n\
         \x20   <file>          Bambu: a .gcode.3mf / .platecycler.3mf bundle\n\
         \x20                   Snapmaker U1: a raw .gcode\n\
         \n\
         OPTIONS:\n\
         \x20   --plate <N>           Bambu plate index inside the bundle (default: 1)\n\
         \x20   --ams <MAP>           Bambu: route filaments to AMS slots, one entry per\n\
         \x20                         filament in T0,T1,… order — e.g. `--ams 0,1,2,3`\n\
         \x20                         (AMS unit 0 slots); `ext` = external spool, `x` = skip\n\
         \x20   --name <NAME>         U1: filename to store on the printer (default: the file's name)\n\
         \x20   --printers-dir <DIR>  printer library dir (default: $XDG_CONFIG_HOME/n3o-slic3r/printers)\n\
         \x20   --wait <SECS>         Bambu: seconds to await a start confirmation (default: 15)\n\
         \x20   --list                list configured instances and exit\n\
         \x20   -h, --help            show this help\n\
         \n\
         The connection kind comes from the instance; the file type must match\n\
         (3MF bundle → Bambu, raw .gcode → Snapmaker). Bambu prints default to\n\
         the external spool; use --ams to route filaments to AMS slots."
    );
}

// ── paths + small helpers ─────────────────────────────────────────────

/// Resolve the printer-library directory: explicit flag, then
/// `N3O_PRINTERS_DIR`, then the GUI's Linux default
/// (`$XDG_CONFIG_HOME/n3o-slic3r/printers`, falling back to
/// `$HOME/.config/...`).
fn resolve_printers_dir(override_dir: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(d) = override_dir {
        return Ok(d);
    }
    if let Some(d) = std::env::var_os("N3O_PRINTERS_DIR").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(d));
    }
    let base = config_dir()
        .ok_or("could not determine the config directory; pass --printers-dir or set $HOME")?;
    Ok(base.join("n3o-slic3r").join("printers"))
}

fn config_dir() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|x| !x.is_empty()) {
        return Some(PathBuf::from(x));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))
}

fn has_ext(p: &Path, ext: &str) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case(ext))
        .unwrap_or(false)
}

fn file_name_of(p: &Path) -> &str {
    p.file_name().and_then(|n| n.to_str()).unwrap_or("file")
}

/// Parse an `--ams` spec into the Bambu `(use_ams, ams_mapping, ams_mapping2)`
/// triple. The spec is a comma list, one entry per filament in `T0,T1,…`
/// order (so its length defines the material count). Each entry:
///   - `N`     → AMS unit 0, slot `N` (`ams_mapping[i]=N`, `mapping2={0,N}`)
///   - `ext`   → external spool (`-1`, `{255,0}`)
///   - `x`     → unused / no tool change for this filament (`-1`, `{255,255}`)
///
/// `use_ams` is true when at least one filament lands on an AMS slot — the
/// same rule the in-app `ams_mapping_for_plate` applies. Single-AMS-unit
/// only (the A1 mini + AMS Lite case), matching the engine-side model.
fn parse_ams_spec(spec: &str) -> Result<(bool, Vec<i8>, Vec<AmsMappingV2>), String> {
    let mut mapping = Vec::new();
    let mut mapping2 = Vec::new();
    let mut any_ams = false;
    for (i, raw) in spec.split(',').enumerate() {
        let tok = raw.trim();
        let label = i + 1; // human-facing filament/material number
        match tok.to_ascii_lowercase().as_str() {
            "ext" | "external" | "e" => {
                mapping.push(-1);
                mapping2.push(AmsMappingV2 {
                    ams_id: 255,
                    slot_id: 0,
                });
            }
            "x" | "-" | "skip" => {
                mapping.push(-1);
                mapping2.push(AmsMappingV2::UNUSED);
            }
            other => {
                let slot: u8 = other.parse().map_err(|_| {
                    format!(
                        "--ams entry {label} (`{tok}`): expected an AMS slot number, `ext`, or `x`"
                    )
                })?;
                if slot > 127 {
                    return Err(format!(
                        "--ams entry {label}: slot {slot} out of range (0–127)"
                    ));
                }
                mapping.push(slot as i8);
                mapping2.push(AmsMappingV2 {
                    ams_id: 0,
                    slot_id: slot,
                });
                any_ams = true;
            }
        }
    }
    if mapping.is_empty() {
        return Err(
            "--ams needs at least one entry (one per filament in the print, T0,T1,… order)"
                .to_owned(),
        );
    }
    Ok((any_ams, mapping, mapping2))
}

/// Human-readable rendering of a resolved AMS map for the progress line.
fn render_ams(mapping: &[i8], mapping2: &[AmsMappingV2]) -> String {
    mapping
        .iter()
        .zip(mapping2)
        .enumerate()
        .map(|(i, (m, m2))| {
            let dest = if *m >= 0 {
                format!("AMS slot {m}")
            } else if *m2 == AmsMappingV2::UNUSED {
                "unused".to_owned()
            } else {
                "external spool".to_owned()
            };
            format!("T{i}→{dest}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    // Default surfaces the driver's own info lines (FTPS/MQTT/HTTP progress)
    // while staying quiet elsewhere; RUST_LOG overrides.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,n3o_slic3r_lib::core::driver=info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ams_all_slots_sets_use_ams_and_per_filament_slots() {
        let (use_ams, mapping, mapping2) = parse_ams_spec("0,1,2,3").unwrap();
        assert!(use_ams);
        assert_eq!(mapping, vec![0, 1, 2, 3]);
        assert_eq!(
            mapping2,
            vec![
                AmsMappingV2 {
                    ams_id: 0,
                    slot_id: 0
                },
                AmsMappingV2 {
                    ams_id: 0,
                    slot_id: 1
                },
                AmsMappingV2 {
                    ams_id: 0,
                    slot_id: 2
                },
                AmsMappingV2 {
                    ams_id: 0,
                    slot_id: 3
                },
            ]
        );
    }

    #[test]
    fn ams_mixes_slot_external_and_skip_with_correct_sentinels() {
        // T0 → AMS slot 2, T1 → external spool, T2 → unused.
        let (use_ams, mapping, mapping2) = parse_ams_spec("2, ext, x").unwrap();
        assert!(use_ams, "an AMS slot is present");
        assert_eq!(mapping, vec![2, -1, -1]);
        assert_eq!(
            mapping2,
            vec![
                AmsMappingV2 {
                    ams_id: 0,
                    slot_id: 2
                },
                AmsMappingV2 {
                    ams_id: 255,
                    slot_id: 0
                }, // ext sentinel
                AmsMappingV2::UNUSED, // {255, 255}
            ]
        );
    }

    #[test]
    fn ams_external_only_leaves_use_ams_false() {
        let (use_ams, mapping, mapping2) = parse_ams_spec("ext,ext").unwrap();
        assert!(!use_ams);
        assert_eq!(mapping, vec![-1, -1]);
        assert!(mapping2.iter().all(|m| m.ams_id == 255 && m.slot_id == 0));
    }

    #[test]
    fn ams_rejects_non_numeric_out_of_range_and_empty() {
        assert!(parse_ams_spec("0,nope").is_err());
        assert!(parse_ams_spec("0,200").is_err()); // > 127 (i8 ceiling)
        assert!(parse_ams_spec("").is_err());
    }
}
