// Printer rail (left) for the Devices fleet monitor — one card per
// configured instance with a live status dot + print progress.

import { useDriverStatus } from "./useDriverStatus";
import type { ConnectionSummary } from "./useDriverConnections";
import type { PrinterInstance } from "../printer/printerInstance";
import { deriveStatus, statusMeta } from "./devicesStatus";

interface RailRowProps {
  instance: PrinterInstance;
  summary: ConnectionSummary | null;
  modelLabel: string;
  selected: boolean;
  onSelect: () => void;
}

function RailRow({
  instance,
  summary,
  modelLabel,
  selected,
  onSelect,
}: RailRowProps): React.JSX.Element {
  const { status } = useDriverStatus(summary?.driverId ?? null);
  const derived = deriveStatus(summary, status);
  const meta = statusMeta(derived.status);
  const printing = derived.status === "printing";
  return (
    <button
      className={`device-card${selected ? " active" : ""}`}
      onClick={onSelect}
      type="button"
      title={`${instance.display_name} — ${meta.label}`}
    >
      <div className="device-card-row1">
        <span className={`device-status-dot ${meta.cls}`} />
        <span className="device-card-name">{instance.display_name}</span>
        <span className={`device-card-state ${meta.cls}`}>{meta.label}</span>
      </div>
      <div className="device-card-row2">
        <span className="device-card-model">{modelLabel}</span>
        {printing && derived.progress != null && (
          <span className="device-card-eta">{Math.round(derived.progress)}%</span>
        )}
      </div>
      {printing && derived.progress != null && (
        <div className="device-card-progress">
          <div
            className="device-card-progress-fill"
            style={{ width: `${derived.progress}%` }}
          />
        </div>
      )}
      {derived.status === "error" && derived.detail && (
        <div className="device-card-error">{derived.detail}</div>
      )}
    </button>
  );
}

interface PrinterRailProps {
  instances: PrinterInstance[];
  connections: Record<string, ConnectionSummary>;
  modelLabel: (i: PrinterInstance) => string;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onAddPrinter: () => void;
}

export function PrinterRail({
  instances,
  connections,
  modelLabel,
  selectedId,
  onSelect,
  onAddPrinter,
}: PrinterRailProps): React.JSX.Element {
  return (
    <aside className="device-rail">
      <div className="device-rail-header">
        <span>Printers</span>
        <span className="device-rail-count">{instances.length}</span>
      </div>
      <div className="device-rail-list">
        {instances.map((inst) => (
          <RailRow
            key={inst.id}
            instance={inst}
            summary={connections[inst.id] ?? null}
            modelLabel={modelLabel(inst)}
            selected={inst.id === selectedId}
            onSelect={() => onSelect(inst.id)}
          />
        ))}
      </div>
      <button className="device-rail-add" onClick={onAddPrinter} type="button">
        <svg width="11" height="11" viewBox="0 0 12 12" fill="none">
          <path
            d="M6 2v8M2 6h8"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinecap="round"
          />
        </svg>
        Add printer
      </button>
    </aside>
  );
}
