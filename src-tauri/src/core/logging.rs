//! Tracing subscriber setup.
//!
//! Single entry point (`init()`) that wires `tracing` to stderr. Filter
//! is driven by `RUST_LOG` (e.g. `RUST_LOG=info,n3o_slic3r_lib=debug`);
//! defaults to `info`. Output format is human-readable text by default;
//! setting `LOG_FORMAT=json` switches to JSON Lines for ingestion by
//! external log pipelines.
//!
//! Safe to call once at app startup. Calling twice is a no-op (the
//! second call's subscriber install is silently ignored — tracing's
//! global subscriber is set-once).

use std::env;

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub fn init() {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let json = env::var("LOG_FORMAT")
        .map(|v| v.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    // Two subscriber shapes; pick at init time. The `set_global_default`
    // call is set-once across the process — guard with .try_init() so a
    // duplicate call (e.g. in tests) is a no-op rather than a panic.
    if json {
        let layer = fmt::layer().json().with_target(true).with_current_span(true);
        let _ = tracing_subscriber::registry()
            .with(env_filter)
            .with(layer)
            .try_init();
    } else {
        let layer = fmt::layer().with_target(true);
        let _ = tracing_subscriber::registry()
            .with(env_filter)
            .with(layer)
            .try_init();
    }
}
