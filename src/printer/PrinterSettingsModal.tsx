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

import { useEffect, useMemo, useRef, useState } from "react";
import { ModalBackdrop, ModalCloseButton } from "../ui/Modal";
import { useModalDismiss } from "../ui/useModalDismiss";
import { AmsPicker } from "./AmsPicker";
import {
  amsUnitsOf,
  deleteInstanceWithReassign,
  updateInstance,
  type ConnectionInfo,
  type InstancePatch,
  type PrinterInstance,
} from "./printerInstance";
import {
  validateBambuConnection,
  validateU1Connection,
  validateConnectionInfo,
  type ConnectionFieldError,
} from "./connectionValidation";
import { usePrinterCatalog } from "./usePrinterCatalog";
import { configForConnection } from "../driver/useDriverConnections";
import { driverTestConnection } from "../driver/invokes";
import type { PlateSnapshot } from "../viewport/types";

/** Bambu printers need a LAN access code; U1 needs a Moonraker port.
 *  Default Moonraker port matches the existing PrinterCredentialsDialog. */
const DEFAULT_U1_PORT = 80;

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

/** Pull the driver kind off the bound printer profile (authored in
 *  the printer's `model.toml`, carried through by the registry).
 *  Single source of truth — no inline string-prefix branches. */
function driverKindFromProfile(
  profile: { driver_kind: "bambu" | "u1" | null } | null,
): "bambu" | "u1" | null {
  return profile?.driver_kind ?? null;
}

// The field-level validators + ConnectionFieldError live in
// `connectionValidation` (shared with the driver reconciler's
// `isConnectionUsable`, so the picker dot and this form agree on
// what "valid" means). Re-exported here so existing importers and
// the unit tests keep reaching them through this module.
export { validateBambuConnection, validateU1Connection };
export type { ConnectionFieldError };

/** Build a `ConnectionInfo` from the draft for a given driver kind,
 *  or `null` for an unknown kind. The single place that knows the
 *  per-kind field layout + trimming — both `handleSave` (to build
 *  the patch) and `validateDraftConnection` (to validate) go through
 *  it, so adding a connection field is one edit, not four. */
function draftToConnection(
  driverKind: "bambu" | "u1" | null,
  draft: Draft,
): ConnectionInfo | null {
  if (driverKind === "bambu") {
    return {
      kind: "bambu",
      host: draft.host.trim(),
      access_code: draft.accessCode.trim(),
    };
  }
  if (driverKind === "u1") {
    return {
      kind: "u1",
      host: draft.host.trim(),
      port: draft.port,
    };
  }
  return null;
}

function validateDraftConnection(
  driverKind: "bambu" | "u1" | null,
  draft: Draft,
): ConnectionFieldError | null {
  const conn = draftToConnection(driverKind, draft);
  return conn == null ? null : validateConnectionInfo(conn);
}


/** Draft shape — mirrors the editable fields. `connection` carries a
 *  superset that gets narrowed at save time per driver kind. */
export interface Draft {
  displayName: string;
  amsUnits: number;
  /** Bambu + U1 shared. */
  host: string;
  /** Bambu only. */
  accessCode: string;
  /** U1 only. */
  port: number;
}

/** Per-field dirty roll-up between an initial and current draft.
 *  Each flag mirrors a single editable surface in the modal; the
 *  view consumes these to mark `.changed` on the corresponding
 *  `.psm-field`. */
export interface DraftChanged {
  displayName: boolean;
  amsUnits: boolean;
  host: boolean;
  accessCode: boolean;
  port: boolean;
}

export function computeChanged(initial: Draft, draft: Draft): DraftChanged {
  return {
    displayName: draft.displayName !== initial.displayName,
    amsUnits: draft.amsUnits !== initial.amsUnits,
    host: draft.host !== initial.host,
    accessCode: draft.accessCode !== initial.accessCode,
    port: draft.port !== initial.port,
  };
}

/** Section-level roll-up of `DraftChanged`. Drives both the
 *  per-tab `psm-nav-dot` indicator and the top-level "is the modal
 *  dirty" check (which gates the Save button + discard overlay). */
export function computeSectionDirty(
  changed: DraftChanged,
  driverKind: "bambu" | "u1" | null,
): { general: boolean; connection: boolean } {
  const connection =
    driverKind === "bambu"
      ? changed.host || changed.accessCode
      : driverKind === "u1"
        ? changed.host || changed.port
        : false;
  return {
    general: changed.displayName || changed.amsUnits,
    connection,
  };
}

export function initialDraft(instance: PrinterInstance): Draft {
  const conn = instance.connection;
  let host = "";
  let accessCode = "";
  let port = DEFAULT_U1_PORT;
  if (conn?.kind === "bambu") {
    host = conn.host;
    accessCode = conn.access_code;
  } else if (conn?.kind === "u1") {
    host = conn.host;
    port = conn.port;
  }
  return {
    displayName: instance.display_name,
    amsUnits: amsUnitsOf(instance),
    host,
    accessCode,
    port,
  };
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
  const [active, setActive] = useState<"general" | "connection">("general");
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

  const sections: { id: "general" | "connection"; label: string; icon: string }[] = [
    { id: "general", label: "General", icon: "⚙" },
    ...(driverKind != null
      ? [{ id: "connection" as const, label: "Connection", icon: "⇄" }]
      : []),
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

        <div className="psm-body">
          <nav className="psm-nav" aria-label="Settings sections">
            {sections.map((s) => (
              <button
                key={s.id}
                type="button"
                className={`psm-nav-item${active === s.id ? " active" : ""}${sectionDirty[s.id] ? " dirty" : ""}`}
                onClick={() => setActive(s.id)}
              >
                <span className="psm-nav-icon">{s.icon}</span>
                <span>{s.label}</span>
                {sectionDirty[s.id] && (
                  <span
                    className="psm-nav-dot"
                    title="Unsaved changes"
                    aria-label="Unsaved changes"
                  />
                )}
              </button>
            ))}
          </nav>

          <section className="psm-content">
            {active === "general" && profile && (
              <GeneralSection
                draft={draft}
                setDraft={setDraft}
                instance={instance}
                profile={profile}
                changed={changed}
                nameInUse={nameInUse}
              />
            )}
            {active === "connection" && driverKind && (
              <ConnectionSection
                driverKind={driverKind}
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
                {(initial.amsUnits - draft.amsUnits) * 4} filament
                binding{(initial.amsUnits - draft.amsUnits) * 4 === 1
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

/** Modal-blocking confirmation card (discard / delete / AMS-shrink).
 *  Owns the overlay + card + alertdialog/aria wiring + backdrop
 *  dismissal once; callers supply the title, body, and action
 *  buttons. Replaces three byte-near-identical inline copies. */
function ConfirmOverlay({
  titleId,
  title,
  onBackdrop,
  actions,
  children,
}: {
  titleId: string;
  title: React.ReactNode;
  onBackdrop: () => void;
  /** The footer buttons (Keep editing / Discard / Delete / …). */
  actions: React.ReactNode;
  /** The body copy (and any inline error note). */
  children: React.ReactNode;
}): React.JSX.Element {
  return (
    <div className="psm-discard-overlay" onClick={onBackdrop}>
      <div
        className="psm-discard-card"
        onClick={(e) => e.stopPropagation()}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={titleId}
      >
        <h3 id={titleId} className="psm-discard-title">
          {title}
        </h3>
        {children}
        <div className="psm-discard-actions">{actions}</div>
      </div>
    </div>
  );
}

function GeneralSection({
  draft,
  setDraft,
  instance,
  profile,
  changed,
  nameInUse,
}: {
  draft: Draft;
  setDraft: React.Dispatch<React.SetStateAction<Draft>>;
  instance: PrinterInstance;
  profile: NonNullable<ReturnType<typeof usePrinterCatalog>["entries"][number]["profile"]>;
  changed: { displayName: boolean; amsUnits: boolean };
  nameInUse: boolean;
}): React.JSX.Element {
  return (
    <div className="psm-section">
      <div className={`psm-field${changed.displayName ? " changed" : ""}`}>
        <label htmlFor="psm-name">Display name</label>
        <div className={`apm-name-input${nameInUse ? " error" : ""}`}>
          <input
            id="psm-name"
            value={draft.displayName}
            onChange={(e) =>
              setDraft((d) => ({ ...d, displayName: e.target.value }))
            }
          />
        </div>
        {nameInUse ? (
          <div className="apm-name-hint error">
            Another printer already uses this name.
          </div>
        ) : (
          <div className="apm-name-hint">
            How this printer shows up in the picker and on plate tabs.
          </div>
        )}
      </div>

      {profile.ams_max > 0 && (
        <div className={`psm-field${changed.amsUnits ? " changed" : ""}`}>
          {/* No outer <label> — AmsPicker's internal `.apm-ams-label`
              already shows the title. The `.changed` accent dot
              picks up that label via the CSS rule that targets
              `.apm-ams-label` inside `.psm-field.changed`. */}
          <AmsPicker
            amsMax={profile.ams_max}
            amsType={profile.ams_type ?? "AMS"}
            value={draft.amsUnits}
            onChange={(n) => setDraft((d) => ({ ...d, amsUnits: n }))}
          />
        </div>
      )}

      <div className="psm-readonly">
        <div className="psm-readonly-row">
          <span>Profile</span>
          <span className="psm-mono">{profile.model}</span>
        </div>
        <div className="psm-readonly-row">
          <span>Build volume</span>
          <span className="psm-mono">
            {profile.build_volume.max[0]} × {profile.build_volume.max[1]} ×{" "}
            {profile.build_volume.max[2]} mm
          </span>
        </div>
        {profile.toolheads.length > 1 && (
          <div className="psm-readonly-row">
            <span>Extruders</span>
            <span className="psm-mono">
              {profile.toolheads.length} toolheads
            </span>
          </div>
        )}
        {instance.extruders.length > 0 && (
          <div className="psm-readonly-row">
            <span>Slots</span>
            <span className="psm-mono">
              {instance.extruders.reduce((sum, e) => sum + e.slots.length, 0)}
            </span>
          </div>
        )}
      </div>
    </div>
  );
}

function ConnectionSection({
  driverKind,
  profileLabel,
  draft,
  setDraft,
  changed,
  fieldError,
  canForget,
  onForget,
  onEdit,
}: {
  driverKind: "bambu" | "u1";
  profileLabel: string;
  draft: Draft;
  setDraft: React.Dispatch<React.SetStateAction<Draft>>;
  changed: { host: boolean; accessCode: boolean; port: boolean };
  fieldError: ConnectionFieldError | null;
  /** True when the instance has a saved connection AND the
   *  user hasn't already clicked Forget this session — drives
   *  the visibility of the Forget button. */
  canForget: boolean;
  onForget: () => void;
  /** Called whenever the user edits a connection field. Cancels a
   *  pending "Forget connection" so re-entering credentials saves
   *  them instead of being overridden by the clear intent. */
  onEdit: () => void;
}): React.JSX.Element {
  const hostError = fieldError?.field === "host" ? fieldError.message : null;
  const portError = fieldError?.field === "port" ? fieldError.message : null;
  const codeError =
    fieldError?.field === "accessCode" ? fieldError.message : null;

  // "Test connection" — spins up a transient backend driver against
  // the current draft and reports the verdict inline. Independent of
  // Save: it never persists. Disabled while the draft is invalid or a
  // test is in flight.
  const [test, setTest] = useState<
    | { kind: "idle" }
    | { kind: "testing" }
    | { kind: "ok" }
    | { kind: "error"; message: string }
  >({ kind: "idle" });
  // Monotonic id identifying the in-flight test. Bumped both when a
  // connection field changes and when a new test starts, so a test
  // that resolves AFTER the user has edited a field is discarded
  // rather than painting a stale verdict against the new draft.
  const testRunRef = useRef(0);
  // A prior verdict is stale the moment any connection field changes;
  // bumping the run id also invalidates any in-flight test.
  useEffect(() => {
    testRunRef.current += 1;
    setTest({ kind: "idle" });
  }, [draft.host, draft.port, draft.accessCode]);
  // Gate the button on the DRAFT's own validity (via the shared
  // helper) rather than the parent's `fieldError`, which is suppressed
  // during Forget mode — so we never test a blanked/half-entered
  // connection.
  const testReady = validateDraftConnection(driverKind, draft) == null;
  const runTest = async (): Promise<void> => {
    const conn = draftToConnection(driverKind, draft);
    if (conn == null) return;
    const runId = (testRunRef.current += 1);
    setTest({ kind: "testing" });
    try {
      await driverTestConnection(configForConnection(conn));
      if (testRunRef.current === runId) setTest({ kind: "ok" });
    } catch (e) {
      if (testRunRef.current === runId) {
        setTest({ kind: "error", message: String(e) });
      }
    }
  };

  return (
    <div className="psm-section">
      <div className={`psm-field${changed.host ? " changed" : ""}`}>
        <label htmlFor="psm-conn-ip">IP address</label>
        <div className={`apm-name-input${hostError ? " error" : ""}`}>
          <input
            id="psm-conn-ip"
            value={draft.host}
            onChange={(e) => {
              onEdit();
              setDraft((d) => ({ ...d, host: e.target.value }));
            }}
            placeholder="e.g. 192.168.1.42"
            inputMode="decimal"
            autoComplete="off"
            spellCheck={false}
          />
        </div>
        {hostError ? (
          <div className="apm-name-hint error">{hostError}</div>
        ) : (
          <div className="apm-name-hint">
            The local network address of your {profileLabel}. Find it on the
            printer&rsquo;s screen under network settings.
          </div>
        )}
      </div>

      {driverKind === "u1" && (
        <div className={`psm-field${changed.port ? " changed" : ""}`}>
          <label htmlFor="psm-conn-port">Port</label>
          <div className={`apm-name-input${portError ? " error" : ""}`}>
            <input
              id="psm-conn-port"
              value={draft.port === 0 ? "" : String(draft.port)}
              onChange={(e) => {
                onEdit();
                const digits = e.target.value.replace(/[^0-9]/g, "");
                const parsed = digits === "" ? 0 : Math.min(65535, Number(digits));
                setDraft((d) => ({ ...d, port: parsed }));
              }}
              placeholder="80"
              inputMode="numeric"
              autoComplete="off"
              spellCheck={false}
            />
          </div>
          {portError ? (
            <div className="apm-name-hint error">{portError}</div>
          ) : (
            <div className="apm-name-hint">
              The port n3o connects on. Defaults to 80; change it only if
              you&rsquo;ve remapped the printer&rsquo;s HTTP interface.
            </div>
          )}
        </div>
      )}

      {driverKind === "bambu" && (
        <div className={`psm-field${changed.accessCode ? " changed" : ""}`}>
          <label htmlFor="psm-conn-code">Access code</label>
          <div className={`apm-name-input${codeError ? " error" : ""}`}>
            <input
              id="psm-conn-code"
              value={draft.accessCode}
              onChange={(e) => {
                onEdit();
                setDraft((d) => ({ ...d, accessCode: e.target.value }));
              }}
              placeholder="8-character code"
              autoComplete="off"
              spellCheck={false}
            />
          </div>
          {codeError ? (
            <div className="apm-name-hint error">{codeError}</div>
          ) : (
            <div className="apm-name-hint">
              Shown on the printer under the LAN-only access settings. Used to
              authenticate this connection.
            </div>
          )}
        </div>
      )}

      {driverKind === "bambu" && (
        <div className="psm-conn-note" role="note">
          <svg
            className="psm-conn-note-ico"
            width="15"
            height="15"
            viewBox="0 0 16 16"
            fill="none"
            aria-hidden="true"
          >
            <path
              d="M8 1.5l5.5 2.4v3.2c0 3.2-2.3 6-5.5 7-3.2-1-5.5-3.8-5.5-7V3.9L8 1.5z"
              stroke="currentColor"
              strokeWidth="1.2"
              strokeLinejoin="round"
            />
            <path
              d="M8 6v3.2M8 11.2h.01"
              stroke="currentColor"
              strokeWidth="1.3"
              strokeLinecap="round"
            />
          </svg>
          <div className="psm-conn-note-body">
            <strong>
              Requires LAN-only mode with Developer Mode enabled.
            </strong>{" "}
            <span>
              On the printer, turn on <em>LAN Only Mode</em> and{" "}
              <em>Developer Mode</em> before connecting — n3o can&rsquo;t reach
              it over the cloud.
            </span>
          </div>
        </div>
      )}

      <div className="psm-conn-test">
        <button
          type="button"
          className="apm-btn"
          onClick={() => void runTest()}
          disabled={!testReady || test.kind === "testing"}
          title="Try connecting with these settings without saving"
        >
          {test.kind === "testing" ? "Testing…" : "Test connection"}
        </button>
        {test.kind === "ok" && (
          <span className="apm-name-hint psm-conn-test-ok">✓ Connected</span>
        )}
        {test.kind === "error" && (
          <span className="apm-name-hint error" title={test.message}>
            ✗ {test.message}
          </span>
        )}
      </div>

      {canForget && (
        <button
          type="button"
          className="psm-delete-trigger"
          onClick={onForget}
          title="Clear the saved connection. The printer stays in your library; you can re-enter credentials any time."
        >
          Forget connection
        </button>
      )}
    </div>
  );
}
