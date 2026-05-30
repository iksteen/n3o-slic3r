//! Read-only filament-loadout Lua binding for the slice hooks.
//!
//! Exposes the active printer's **bound** filament loadout — what
//! filament each physical slot is bound to, as resolved from the
//! [`PrinterInstance`] at slice time. This is the slice-time
//! material→slot mapping, *not* the live driver report: no `loaded`
//! flag, no AMS/RFID readout, no mismatch state (those need the live
//! driver status + the mismatch detector, neither of which is in scope
//! at the slice worker). Plugins use this to be material-aware in a
//! purely declarative way, e.g. "only append the swap macro if every
//! slot is bound" or "tag the G-code with the loaded materials."
//!
//! Handed to `on_pre_slice` / `on_post_slice` as a third positional
//! argument (`filament`). One-way read into host state: the
//! `FilamentLoadout` lives Rust-side behind an `Arc` and is **never**
//! reachable from Lua, so a plugin can never write filament state back
//! into the slice (FR-PL-7). The `filament` handle is immutable
//! userdata — assignment raises. The per-slot / printer tables returned
//! by `slots()`/`slot()`/`printer()` are fresh snapshots whose `=`
//! assignment path raises via `__newindex`; the sandbox strips `rawset`
//! / `getmetatable` / `setmetatable` (see `sandbox.rs`), so that guard
//! can't be bypassed — the snapshots are effectively immutable from Lua.
//!
//! ```lua
//! function on_post_slice(gcode, plate, filament)
//!   for _, s in ipairs(filament:slots()) do
//!     if not s.bound then return end          -- bail if any slot empty
//!   end
//!   gcode:append("; all " .. filament:count() .. " slots bound")
//! end
//! ```

use std::sync::Arc;

use mlua::{Lua, MetaMethod, Result as LuaResult, Table, UserData, UserDataMethods, Value};

use crate::core::filament;
use crate::core::printer::{FeedKind, PrinterInstance};

/// One physical filament slot's bound identity — a slice-time snapshot,
/// not live driver state.
#[derive(Debug, Clone)]
pub struct SlotInfo {
    /// 1-based ordinal across all extruders, in flat slot order. Equals
    /// the libslic3r-side filament index, so material `index` emits
    /// `T<index - 1>`.
    pub index: usize,
    /// 0-based physical extruder this slot feeds.
    pub extruder: u8,
    /// 0-based slot within that extruder.
    pub slot: u8,
    /// `"direct"` or `"ams"` — the slot's feed path.
    pub feed: &'static str,
    /// Bound filament identity slug, or `None` when the slot is unbound.
    pub identity: Option<String>,
    /// Material family of the bound filament (`"PLA"`, `"PETG"`, …),
    /// resolved from the bundled catalog. `None` when unbound or the
    /// identity isn't in the catalog.
    pub base_type: Option<String>,
    /// Spool color — the per-slot binding color if set, else the
    /// catalog profile's color. `None` when neither is known.
    pub color: Option<String>,
    /// Vendor of the bound filament, when the catalog records one.
    pub vendor: Option<String>,
}

/// Read-only snapshot of a printer's bound filament loadout, assembled
/// from a [`PrinterInstance`] at slice time. Handed to the slice hooks
/// as the `filament` argument.
#[derive(Debug, Clone, Default)]
pub struct FilamentLoadout {
    pub printer_model: String,
    pub toolhead_count: usize,
    pub slots: Vec<SlotInfo>,
}

impl FilamentLoadout {
    /// Build from a resolved instance. Walks every extruder's slots in
    /// flat order, resolving each bound `filament_identity` to its
    /// type/vendor via the bundled filament catalog; color prefers the
    /// per-slot binding color and falls back to the catalog color.
    pub fn from_instance(
        instance: &PrinterInstance,
        printer_model: String,
        toolhead_count: usize,
    ) -> Self {
        let mut slots = Vec::new();
        let mut index = 1;
        for (e, extruder) in instance.extruders.iter().enumerate() {
            for (s, binding) in extruder.slots.iter().enumerate() {
                let profile = binding
                    .filament_identity
                    .as_deref()
                    .and_then(filament::lookup);
                let base_type = profile.as_ref().map(|p| p.base_type.clone());
                let vendor = profile.as_ref().and_then(|p| p.vendor.clone());
                let color = binding
                    .color
                    .clone()
                    .or_else(|| profile.as_ref().and_then(|p| p.color.clone()));
                slots.push(SlotInfo {
                    index,
                    extruder: e as u8,
                    slot: s as u8,
                    feed: match binding.feed {
                        FeedKind::Direct => "direct",
                        FeedKind::Ams => "ams",
                    },
                    identity: binding.filament_identity.clone(),
                    base_type,
                    color,
                    vendor,
                });
                index += 1;
            }
        }
        Self {
            printer_model,
            toolhead_count,
            slots,
        }
    }
}

/// Lua handle over a [`FilamentLoadout`]. Read-only: methods return
/// fresh snapshot tables, and assigning a field raises.
pub struct FilamentHandle {
    inner: Arc<FilamentLoadout>,
}

impl FilamentHandle {
    pub fn new(loadout: FilamentLoadout) -> Self {
        Self {
            inner: Arc::new(loadout),
        }
    }
}

/// The error raised by the `=` assignment path on a snapshot table.
fn read_only_err() -> mlua::Error {
    mlua::Error::RuntimeError("filament state is read-only".into())
}

/// Wrap `data` in the shared read-only proxy (see [`super::read_only`]).
fn read_only_proxy(lua: &Lua, data: Table) -> LuaResult<Table> {
    super::read_only(lua, data, "filament state")
}

/// Build the read-only Lua table for one slot. Unbound/unknown fields
/// are simply absent (read as `nil`), matching the "identity fields nil
/// when unbound" contract.
fn slot_proxy(lua: &Lua, s: &SlotInfo) -> LuaResult<Table> {
    let t = lua.create_table()?;
    t.set("index", s.index)?;
    // Surface positional coords 1-based (Lua convention; matches the
    // frontend's T1..TN / AMS:1.. labels).
    t.set("extruder", s.extruder as usize + 1)?;
    t.set("slot", s.slot as usize + 1)?;
    t.set("feed", s.feed)?;
    t.set("bound", s.identity.is_some())?;
    if let Some(v) = &s.identity {
        t.set("identity", v.clone())?;
    }
    if let Some(v) = &s.base_type {
        t.set("type", v.clone())?;
    }
    if let Some(v) = &s.color {
        t.set("color", v.clone())?;
    }
    if let Some(v) = &s.vendor {
        t.set("vendor", v.clone())?;
    }
    read_only_proxy(lua, t)
}

impl UserData for FilamentHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // filament:slots() -> array of read-only slot tables (1-based).
        methods.add_method("slots", |lua, this, ()| {
            let arr = lua.create_table()?;
            for s in &this.inner.slots {
                arr.push(slot_proxy(lua, s)?)?;
            }
            Ok(arr)
        });
        // filament:slot(i) -> the 1-based i-th slot, or nil.
        methods.add_method("slot", |lua, this, i: i64| {
            if i < 1 {
                return Ok(None);
            }
            match this.inner.slots.get((i - 1) as usize) {
                Some(s) => Ok(Some(slot_proxy(lua, s)?)),
                None => Ok(None),
            }
        });
        // filament:count() -> number of physical slots.
        methods.add_method("count", |_, this, ()| Ok(this.inner.slots.len()));
        // filament:printer() -> { model, toolhead_count } (read-only).
        methods.add_method("printer", |lua, this, ()| {
            let t = lua.create_table()?;
            t.set("model", this.inner.printer_model.clone())?;
            t.set("toolhead_count", this.inner.toolhead_count)?;
            read_only_proxy(lua, t)
        });
        // Assigning a field on the handle itself is a read-only error.
        methods.add_meta_method(
            MetaMethod::NewIndex,
            |_, _this, (_k, _v): (Value, Value)| -> LuaResult<()> { Err(read_only_err()) },
        );
    }
}
