// DevicesView — fleet monitor (ported from docs/dev/design/DevicesView.jsx).
//
// A top-level mode (alongside Prepare / Preview) that replaces the
// topbar PrinterPanel's monitoring role: a printer rail on the left
// lists every configured instance with a live status dot + print
// progress; selecting one fills the monitor on the right (camera,
// temps, current job, filament loadout, pause/resume/stop).
//
// This file is the orchestrator: the rail (PrinterRail), the monitor
// (DeviceMonitor), and their building blocks (camera / stats / loadout /
// job / status derivation) live in sibling modules.
//
// Data wiring vs the mockup:
//   - status comes from useDriverConnections (per-instance summary) +
//     useDriverStatus (live job/temps/extra) rather than a fake map.
//   - loadout is the instance's synced slot bindings (flattenSlots),
//     resolved to display via the filament catalog — the same data the
//     slot chip strip shows. The driver-reported "currently engaged"
//     slot is highlighted (DeviceMonitor's activeSlot).
//   - webcam streams live for backends with camera support (Bambu LAN
//     today) via CameraPanel / useCameraStream; other backends show the
//     disabled-camera state.
//   - jump-to-originating-plate is omitted (no job→plate mapping yet).

import { useMemo } from "react";
import type { ConnectionSummary } from "./useDriverConnections";
import type { PrinterInstance } from "../printer/printerInstance";
import { usePrinterCatalog } from "../printer/usePrinterCatalog";
import { PrinterRail } from "./PrinterRail";
import { DeviceMonitor } from "./DeviceMonitor";

// ───────── Model label ─────────

function useModelLabels(): (instance: PrinterInstance) => string {
  const catalog = usePrinterCatalog();
  return useMemo(() => {
    const byIdentity = new Map(
      catalog.entries.map((e) => [e.identity, e.profile] as const),
    );
    return (instance: PrinterInstance) => {
      const profile = byIdentity.get(instance.vendor_profile_ref);
      if (!profile) return instance.vendor_profile_ref;
      return `${profile.brand} ${profile.model}`;
    };
  }, [catalog.entries]);
}

// ───────── Root ─────────

export interface DevicesViewProps {
  instances: PrinterInstance[];
  connections: Record<string, ConnectionSummary>;
  /** Selected printer instance id (controlled). App owns this state so it
   *  survives DevicesView's unmount on every Prepare/Preview tab switch, and
   *  so a Send can pre-select the destination printer before the view mounts.
   *  `null` (or an id no longer in `instances`) falls back to the first
   *  printer. */
  selectedId: string | null;
  /** Commit a rail selection up to App's owned state. */
  onSelectId: (id: string) => void;
  onAddPrinter: () => void;
  onEditPrinter: (id: string) => void;
}

export function DevicesView({
  instances,
  connections,
  selectedId,
  onSelectId,
  onAddPrinter,
  onEditPrinter,
}: DevicesViewProps): React.JSX.Element {
  const modelLabel = useModelLabels();

  // Default to the first printer; keep the selection valid as the
  // instance list changes (e.g. the selected printer is deleted).
  const selected =
    instances.find((i) => i.id === selectedId) ?? instances[0] ?? null;

  return (
    <div className="devices-view">
      <PrinterRail
        instances={instances}
        connections={connections}
        modelLabel={modelLabel}
        selectedId={selected?.id ?? null}
        onSelect={onSelectId}
        onAddPrinter={onAddPrinter}
      />
      <DeviceMonitor
        instance={selected}
        summary={selected ? (connections[selected.id] ?? null) : null}
        modelLabel={selected ? modelLabel(selected) : ""}
        onEditPrinter={onEditPrinter}
      />
    </div>
  );
}
