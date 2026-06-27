import { AmsPicker } from "./AmsPicker";
import type { PrinterInstance } from "./printerInstance";
import { usePrinterCatalog } from "./usePrinterCatalog";
import type { Draft } from "./printerSettingsHelpers";

export function GeneralSection({
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
