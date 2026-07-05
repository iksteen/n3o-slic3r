// Per-printer settings modal — reached by clicking the cog next to a
// printer in the picker.
//
// V1 scope: General + Connection tabs, plus delete and close/save in
// the footer. Start G-code / End G-code / Machine limits are
// declared in the mockup but deferred — the sections array
// conditionally includes them once their fields are wired.
//
// Draft-state pattern: every edit lands in local state; "Save
// changes" enabled only when dirty AND valid. Per-field dirty
// markers (a `.changed` class on `.psm-field` / `.psm-limit-cell`)
// and per-section dirty dots (`psm-nav-dot` next to the nav label)
// let the user find their pending edits.
//
// requestClose() funnels every close request (Esc, backdrop click,
// close-button, footer Cancel). With unsaved edits, it shows the
// `psm-discard-overlay` card (Keep editing / Discard / Save & close).

import { useEffect, useMemo, useState } from "react";
import { ModalBackdrop, ModalCloseButton } from "../ui/Modal";
import { useModalDismiss } from "../ui/useModalDismiss";
import {
  deleteInstanceWithReassign,
  updateInstance,
  type InstancePatch,
  type PrinterInstance,
} from "./printerInstance";
import { usePrinterCatalog } from "./usePrinterCatalog";
import type { PlateSnapshot } from "../viewport/types";
import { useMachineOptions, useExtruderOptions } from "../settings/resolve";
import { categorize } from "../settings/nav/categories";
import { setMachineOverride, resolvedInstanceConfig } from "./printerInstance";
import { MachineSettingsSection } from "./MachineSettingsSection";
import { ExtruderSettingsSection } from "./ExtruderSettingsSection";
import {
  computeChanged,
  computeSectionDirty,
  driverKindFromProfile,
  draftToConnection,
  initialDraft,
  MACHINE_PAGE_ORDER,
  orderGroupsOtherLast,
  notesLast,
  validateDraftConnection,
  type Draft,
} from "./printerSettingsHelpers";
import { ConfirmOverlay } from "./PrinterSettingsConfirmOverlay";
import { GeneralSection } from "./PrinterSettingsGeneralSection";
import { ConnectionSection } from "./PrinterSettingsConnectionSection";
import { PluginsSection } from "./PrinterSettingsPluginsSection";

export interface PrinterSettingsModalProps {
  /** The instance the cog opened. The modal scopes everything to it. */
  instance: PrinterInstance;
  /** All registered instances. Drives the name-uniqueness check and
   *  the delete-fallback (plates rebind to the first other one). */
  instances: PrinterInstance[];
  /** All plates in the current project. On delete, plates bound to
   *  this instance get rebound to the first remaining instance. */
  plates: PlateSnapshot[];
  onClose: () => void;
}

export function PrinterSettingsModal({
  instance,
  instances,
  plates,
  onClose,
}: PrinterSettingsModalProps): React.JSX.Element | null {
  const catalog = usePrinterCatalog();
  const profile = useMemo(
    () =>
      catalog.entries.find((e) => e.identity === instance.vendor_profile_ref)
        ?.profile ?? null,
    [catalog.entries, instance.vendor_profile_ref],
  );
  const driverKind = driverKindFromProfile(profile);

  const [draft, setDraft] = useState<Draft>(() => initialDraft(instance));
  // Top-level tab: "general" (general/connection/plugins), "machine" (the
  // machine-wide settings categories), or "ext:<n>" (per-toolhead settings).
  const [topTab, setTopTab] = useState<string>("general");
  // The selected left-nav item within the active top tab. Base sections
  // are fixed ids; machine-settings categories add their (dynamic,
  // scraped) page titles as further nav ids — hence `string`.
  const [active, setActive] = useState<string>("general");
  /** Which modal-blocking confirmation overlay is showing, if any.
   *  One union instead of three mutually-exclusive booleans —
   *  delete / discard / amsShrink never co-occur, so a single state
   *  makes the invariant explicit and they all render through one
   *  `ConfirmOverlay`. */
  const [overlay, setOverlay] = useState<
    "delete" | "discard" | "amsShrink" | null
  >(null);
  /** True after the user clicks "Forget connection" — clears the
   *  draft fields AND signals handleSave to persist `null` instead
   *  of constructing a ConnectionInfo from empty draft values.
   *  Bypasses the connection-field validator gate
   *  (empty-host-rejects-save) for this one path. Reset when the
   *  instance changes (e.g. modal opens for a different printer). */
  const [forgetConnection, setForgetConnection] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  // Re-seed the draft if the modal is reopened for a different
  // instance (parent swaps the prop). Resets all UI state too.
  useEffect(() => {
    setDraft(initialDraft(instance));
    setTopTab("general");
    setActive("general");
    setOverlay(null);
    setSaveError(null);
    setForgetConnection(false);
  }, [instance.id]);

  // Per-field changed flags + per-section roll-up. The mockup uses
  // these to mark the edited tabs (`psm-nav-dot`) and the edited
  // fields (`.changed`) so the user can find unsaved edits.
  const initial = useMemo(() => initialDraft(instance), [instance]);
  const changed = useMemo(() => computeChanged(initial, draft), [initial, draft]);
  const sectionDirty = useMemo(
    () => computeSectionDirty(changed, driverKind),
    [changed, driverKind],
  );
  const dirty = sectionDirty.general || sectionDirty.connection;

  // Machine + per-extruder (Printer-bucket) settings — auto-populated from
  // libslic3r and grouped by Orca's TabPrinter pages/optgroups. Each group
  // is a left-nav category; overrides persist live to `config_overrides`.
  const { options: machineOptions } = useMachineOptions(profile);
  const { options: extruderOptions } = useExtruderOptions(profile);
  // Machine pages follow a curated Orca order (MACHINE_PAGE_ORDER): the
  // display-order scrape can't place the `build_unregular_pages` pages
  // (Motion ability, Multimaterial), which render before Notes despite being
  // coded after it, so first-appearance would sort Notes ahead of them.
  // notesLast then guarantees Notes is terminal even if a page category isn't
  // in the curated list.
  const machineGroups = useMemo(
    () =>
      notesLast(orderGroupsOtherLast(categorize(machineOptions, MACHINE_PAGE_ORDER))),
    [machineOptions],
  );
  // The per-extruder option set is identical across toolheads; only the
  // displayed vector index differs. Rendered as one page (ExtruderSettingsSection
  // groups by optgroup internally), mirroring Orca's single Extruder page.
  const hasExtruderOptions = extruderOptions.length > 0;
  const extruderCount = instance.extruders.length;
  const [resolved, setResolved] = useState<Record<string, string>>({});
  // Gates the Silent column on the machine-limits rows. libslic3r serializes
  // the bool as "1"/"0"; treat anything truthy as on.
  const silentMode =
    resolved["silent_mode"] === "1" || resolved["silent_mode"] === "true";
  // Re-resolve when the instance's overrides change (a `when`-gated value
  // elsewhere can shift); keyed on a serialization so it's value-stable.
  const overridesKey = JSON.stringify(instance.config_overrides);
  useEffect(() => {
    let cancelled = false;
    resolvedInstanceConfig(instance.id)
      .then((r) => {
        if (!cancelled) setResolved(r);
      })
      .catch((e) => console.error("[printer] resolved config failed", e));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [instance.id, overridesKey]);
  const onSetMachine = (key: string, value: string): void => {
    setMachineOverride(instance.id, key, value).catch((e) =>
      setSaveError(String(e)),
    );
  };
  const onClearMachine = (key: string): void => {
    setMachineOverride(instance.id, key, null).catch((e) =>
      setSaveError(String(e)),
    );
  };
  // Per-tab left-nav groups for the non-"general" tabs.
  const isExtruderTab = topTab.startsWith("ext:");
  const extruderIndex = isExtruderTab ? Number(topTab.slice(4)) : 0;
  // The extruder tab is a single page (no sub-nav); only the machine tab
  // has a left-nav of pages.
  const currentGroups = topTab === "machine" ? machineGroups : [];
  // Categories load async; if `active` isn't (yet) one of the current tab's
  // groups, fall back to the first so the tab always shows content.
  const navActive = currentGroups.some((g) => g.id === active)
    ? active
    : (currentGroups[0]?.id ?? "");

  // Switch top tab and land on a valid left-nav item for it.
  const selectTopTab = (tab: string): void => {
    setTopTab(tab);
    if (tab === "general") setActive("general");
    else if (tab === "machine") setActive(machineGroups[0]?.id ?? "");
    // Extruder tabs have no sub-nav; nothing to activate.
  };

  // Name validation: trim, reject empty, reject if matches another
  // instance's display_name (case-sensitive — display names are
  // exact tokens on plate tabs).
  const trimmedName = draft.displayName.trim();
  const nameInUse =
    trimmedName.length > 0 &&
    instances.some((i) => i.id !== instance.id && i.display_name === trimmedName);
  // Connection-section validation. Surface an error whenever the
  // section is being edited OR the instance already has a stored
  // connection — so a pre-existing invalid/partial connection (a
  // hand-edited or partially-migrated config) shows up in the
  // Connection tab on open instead of silently degrading to a grey
  // picker dot. A printer with no connection yet isn't nagged until
  // the user starts entering one. Forget-mode bypasses entirely —
  // clearing IS the intent.
  const hasStoredConnection = instance.connection != null;
  const connectionError =
    !forgetConnection && (sectionDirty.connection || hasStoredConnection)
      ? validateDraftConnection(driverKind, draft)
      : null;
  // The connection error only GATES save when the connection section
  // is actually being changed — editing just the display name on a
  // printer whose stored connection is invalid still saves the
  // rename (the error stays visible as a warning, it just doesn't
  // block the unrelated edit).
  const canSave =
    trimmedName.length > 0 &&
    !nameInUse &&
    !(sectionDirty.connection && connectionError) &&
    dirty &&
    !saving;

  // Funnel for every close-request route (Esc, backdrop, close
  // button, footer Cancel). With unsaved edits, show the discard
  // overlay instead of dismissing immediately.
  const requestClose = (): void => {
    if (dirty) {
      setOverlay("discard");
      return;
    }
    onClose();
  };

  // Layered Esc handling: any open overlay dismisses first;
  // otherwise route through the dirty-aware requestClose.
  useModalDismiss(
    () => {
      if (overlay != null) {
        setOverlay(null);
        return;
      }
      requestClose();
    },
    { active: true },
  );

  // Persist the draft. Each mutator is independent; if one fails,
  // surface the error but commit whatever already succeeded. Driver
  // connect/disconnect is NOT triggered here — the
  // useDriverConnections hook reacts to the resulting
  // printer:instance_changed event and reconciles the live driver
  // registry to match the persisted connection.
  const handleSave = async (
    opts: { confirmAmsShrink?: boolean } = {},
  ): Promise<boolean> => {
    if (!canSave) return false;
    // AMS-units downgrade is destructive — every dropped AMS unit
    // takes 4 slot bindings with it. Gate behind a confirm
    // overlay (mirrors confirmingDelete's shape) on the first
    // save attempt; the overlay's "Drop bindings" button calls
    // handleSave again with confirmAmsShrink=true to bypass.
    if (
      changed.amsUnits &&
      draft.amsUnits < initial.amsUnits &&
      !opts.confirmAmsShrink
    ) {
      setOverlay("amsShrink");
      return false;
    }
    setSaving(true);
    setSaveError(null);
    try {
      // One atomic update — backend takes a patch, applies all
      // changed fields under one lock, persists once, emits a
      // single `printer:instance_changed`. No partial-success
      // window between display-name and connection writes.
      const patch: InstancePatch = {};
      if (changed.displayName) patch.displayName = trimmedName;
      if (changed.amsUnits) patch.amsUnits = draft.amsUnits;
      if (sectionDirty.connection) {
        if (forgetConnection) {
          patch.clearConnection = true;
        } else {
          const newConn = draftToConnection(driverKind, draft);
          if (newConn != null) patch.connection = newConn;
        }
      }
      await updateInstance(instance.id, patch);
      setSaving(false);
      // Persist-and-close. The driver reconciler (useDriverConnections)
      // reacts to the resulting printer:instance_changed event and
      // registers/replaces/unregisters the live driver asynchronously;
      // the picker dot and PrinterPanel surface connect/fail status, so
      // the modal doesn't wait on it.
      return true;
    } catch (e) {
      setSaveError(String(e));
      setSaving(false);
      return false;
    }
  };

  // Delete path: atomic via the backend composite. Rebinds bound
  // plates to the fallback (or unbinds them when this is the
  // last printer — fallback is null) and deletes the instance in
  // one registry+project lock. No frontend partial-commit window.
  const fallback = instances.find((i) => i.id !== instance.id) ?? null;
  const handleDelete = async (): Promise<void> => {
    try {
      const plateIds = plates
        .filter((p) => p.printer_instance_id === instance.id)
        .map((p) => p.plate_id);
      await deleteInstanceWithReassign(
        instance.id,
        fallback?.id ?? null,
        plateIds,
      );
      onClose();
    } catch (e) {
      setSaveError(String(e));
      setOverlay(null);
    }
  };

  const sections: {
    id: "general" | "connection" | "plugins";
    label: string;
    icon: string;
    dirty: boolean;
  }[] = [
    { id: "general", label: "General", icon: "⚙", dirty: sectionDirty.general },
    ...(driverKind != null
      ? [
          {
            id: "connection" as const,
            label: "Connection",
            icon: "⇄",
            dirty: sectionDirty.connection,
          },
        ]
      : []),
    // Plugins persist live (each toggle hits the backend immediately), so
    // they're outside the draft/Save flow — never "dirty".
    { id: "plugins" as const, label: "Plugins", icon: "🧩", dirty: false },
  ];

  return (
    <ModalBackdrop
      onDismiss={requestClose}
      cardClassName="printer-settings-modal"
      ariaLabelledBy="psm-title"
    >
        <header className="psm-header">
          <div className="psm-header-mark" data-brand={profile?.brand}>
            <span>{profile?.brand_short ?? "?"}</span>
          </div>
          <div className="psm-header-text">
            <h2 id="psm-title">{instance.display_name}</h2>
            <p>
              Based on{" "}
              <span className="psm-profile-label">
                {profile?.model ?? instance.vendor_profile_ref}
              </span>
              {profile && (
                <>
                  &nbsp;·&nbsp;
                  <span className="psm-mono">
                    {profile.build_volume.max[0]} × {profile.build_volume.max[1]} ×{" "}
                    {profile.build_volume.max[2]} mm
                  </span>
                </>
              )}
            </p>
          </div>
          <ModalCloseButton onClick={requestClose} />
        </header>

        <div className="psm-tabs" role="tablist" aria-label="Printer settings">
          <button
            type="button"
            role="tab"
            aria-selected={topTab === "general"}
            className={`psm-tab${topTab === "general" ? " active" : ""}${
              dirty ? " dirty" : ""
            }`}
            onClick={() => selectTopTab("general")}
          >
            General
            {dirty && <span className="psm-nav-dot" aria-label="Unsaved changes" />}
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={topTab === "machine"}
            className={`psm-tab${topTab === "machine" ? " active" : ""}`}
            onClick={() => selectTopTab("machine")}
            disabled={machineGroups.length === 0}
          >
            Machine
          </button>
          {hasExtruderOptions &&
            Array.from({ length: extruderCount }, (_, i) => {
              const id = `ext:${i}`;
              return (
                <button
                  key={id}
                  type="button"
                  role="tab"
                  aria-selected={topTab === id}
                  className={`psm-tab${topTab === id ? " active" : ""}`}
                  onClick={() => selectTopTab(id)}
                >
                  {extruderCount > 1 ? `Extruder ${i + 1}` : "Extruder"}
                </button>
              );
            })}
        </div>

        <div className={`psm-body${isExtruderTab ? " single-col" : ""}`}>
          {/* Extruder tabs are a single page — no left-nav. */}
          {!isExtruderTab && (
          <nav className="psm-nav" aria-label="Settings sections">
            {topTab === "general"
              ? sections.map((s) => (
                  <button
                    key={s.id}
                    type="button"
                    className={`psm-nav-item${active === s.id ? " active" : ""}${s.dirty ? " dirty" : ""}`}
                    onClick={() => setActive(s.id)}
                  >
                    <span className="psm-nav-icon">{s.icon}</span>
                    <span>{s.label}</span>
                    {s.dirty && (
                      <span
                        className="psm-nav-dot"
                        title="Unsaved changes"
                        aria-label="Unsaved changes"
                      />
                    )}
                  </button>
                ))
              : currentGroups.map((g) => (
                  <button
                    key={g.id}
                    type="button"
                    className={`psm-nav-item${navActive === g.id ? " active" : ""}`}
                    onClick={() => setActive(g.id)}
                  >
                    <span className="psm-nav-icon">{g.icon}</span>
                    <span>{g.name}</span>
                  </button>
                ))}
          </nav>
          )}

          <section className="psm-content">
            {topTab === "general" && active === "general" && profile && (
              <GeneralSection
                draft={draft}
                setDraft={setDraft}
                instance={instance}
                profile={profile}
                changed={changed}
                nameInUse={nameInUse}
              />
            )}
            {topTab === "general" && active === "connection" && driverKind && (
              <ConnectionSection
                driverKind={driverKind}
                instanceId={instance.id}
                profileLabel={profile?.model ?? instance.vendor_profile_ref}
                draft={draft}
                setDraft={setDraft}
                changed={changed}
                fieldError={connectionError}
                canForget={instance.connection != null && !forgetConnection}
                onForget={() => {
                  setForgetConnection(true);
                  // Blank only the credential fields the user re-enters;
                  // preserve `port` so a U1 with a remapped Moonraker
                  // port doesn't silently revert to 80 if the user
                  // changes their mind and re-types only the host.
                  setDraft((d) => ({
                    ...d,
                    host: "",
                    accessCode: "",
                  }));
                }}
                onEdit={() => setForgetConnection(false)}
              />
            )}
            {topTab === "general" && active === "plugins" && (
              <PluginsSection
                instance={instance}
                printerModel={profile?.model ?? null}
              />
            )}
            {topTab === "machine" &&
              machineGroups.map((g) =>
              navActive === g.id ? (
                <MachineSettingsSection
                  key={g.id}
                  settings={g.settings}
                  overrides={instance.config_overrides}
                  resolved={resolved}
                  silentMode={silentMode}
                  onSet={onSetMachine}
                  onClear={onClearMachine}
                />
              ) : null,
            )}
            {isExtruderTab && (
              <ExtruderSettingsSection
                extruderIndex={extruderIndex}
                settings={extruderOptions}
                overrides={instance.config_overrides}
                resolved={resolved}
                onSet={onSetMachine}
                onClear={onClearMachine}
              />
            )}
          </section>
        </div>

        <footer className="psm-footer">
          <button
            className="psm-delete-trigger"
            onClick={() => setOverlay("delete")}
            type="button"
            title="Delete this printer"
          >
            <svg width="12" height="12" viewBox="0 0 14 14" fill="none">
              <path
                d="M3 4h8M5 4V2.5a1 1 0 0 1 1-1h2a1 1 0 0 1 1 1V4M4.5 4l.5 7a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1l.5-7"
                stroke="currentColor"
                strokeWidth="1.2"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
            Delete printer
          </button>
          <div className="apm-actions">
            {saveError && (
              <span
                className="apm-name-hint error"
                style={{ marginRight: 8 }}
              >
                {saveError}
              </span>
            )}
            <button
              className="apm-btn"
              onClick={requestClose}
              type="button"
            >
              {dirty ? "Cancel" : "Close"}
            </button>
            <button
              className="apm-btn primary"
              onClick={() => {
                void handleSave().then((ok) => {
                  if (ok) onClose();
                });
              }}
              disabled={!canSave}
              type="button"
            >
              {saving ? "Saving…" : "Save changes"}
            </button>
          </div>
        </footer>

        {overlay === "delete" && (
          <ConfirmOverlay
            titleId="psm-delete-title"
            title={<>Delete &ldquo;{instance.display_name}&rdquo;?</>}
            onBackdrop={() => setOverlay(null)}
            actions={
              <>
                <button
                  className="apm-btn"
                  onClick={() => setOverlay(null)}
                  type="button"
                >
                  Keep editing
                </button>
                <button
                  className="apm-btn danger"
                  onClick={() => {
                    void handleDelete();
                  }}
                  type="button"
                >
                  Delete printer
                </button>
              </>
            }
          >
            <p className="psm-discard-body">
              {fallback ? (
                <>
                  Plates using this printer will be reassigned to{" "}
                  <strong>{fallback.display_name}</strong>. The
                  printer&rsquo;s saved settings, connection, and
                  slot bindings will be removed.
                </>
              ) : (
                <>
                  This is your last printer. Deleting it sends the
                  workspace back to the <strong>add-printer
                  screen</strong> — pick a new one to start
                  printing again.
                </>
              )}
            </p>
          </ConfirmOverlay>
        )}

        {overlay === "discard" && (
          <ConfirmOverlay
            titleId="psm-discard-title"
            title="Unsaved changes"
            onBackdrop={() => setOverlay(null)}
            actions={
              <>
                <button
                  className="apm-btn"
                  onClick={() => setOverlay(null)}
                  type="button"
                >
                  Keep editing
                </button>
                <button
                  className="apm-btn danger"
                  onClick={() => {
                    setOverlay(null);
                    onClose();
                  }}
                  type="button"
                >
                  Discard changes
                </button>
                <button
                  className="apm-btn primary"
                  onClick={() => {
                    void handleSave().then((ok) => {
                      if (ok) {
                        setOverlay(null);
                        onClose();
                      }
                      // If !ok, saveError is now set; the overlay
                      // stays mounted and surfaces it inline above.
                    });
                  }}
                  disabled={!canSave}
                  type="button"
                >
                  Save &amp; close
                </button>
              </>
            }
          >
            <p className="psm-discard-body">
              You have unsaved changes to <strong>{instance.display_name}</strong>.
              Closing now will discard them.
            </p>
            {saveError && (
              <p
                className="apm-name-hint error"
                style={{ marginTop: -8, marginBottom: 12 }}
                role="alert"
              >
                {saveError}
              </p>
            )}
          </ConfirmOverlay>
        )}

        {overlay === "amsShrink" && (
          <ConfirmOverlay
            titleId="psm-ams-shrink-title"
            title="Reduce AMS units?"
            onBackdrop={() => setOverlay(null)}
            actions={
              <>
                <button
                  className="apm-btn"
                  onClick={() => setOverlay(null)}
                  type="button"
                >
                  Keep editing
                </button>
                <button
                  className="apm-btn danger"
                  onClick={() => {
                    setOverlay(null);
                    void handleSave({ confirmAmsShrink: true }).then(
                      (ok) => {
                        if (ok) onClose();
                      },
                    );
                  }}
                  type="button"
                >
                  Drop bindings
                </button>
              </>
            }
          >
            <p className="psm-discard-body">
              You&rsquo;re reducing from{" "}
              <strong>{initial.amsUnits}</strong> to{" "}
              <strong>{draft.amsUnits}</strong> AMS units. This will
              drop{" "}
              <strong>
                {(initial.amsUnits - draft.amsUnits) *
                  (profile?.ams_slots_per_unit ?? 0)}{" "}
                filament binding
                {(initial.amsUnits - draft.amsUnits) *
                  (profile?.ams_slots_per_unit ?? 0) ===
                1
                  ? ""
                  : "s"}
              </strong>{" "}
              from the AMS slots — the spool/color customizations on
              those slots will be lost.
            </p>
          </ConfirmOverlay>
        )}
    </ModalBackdrop>
  );
}
