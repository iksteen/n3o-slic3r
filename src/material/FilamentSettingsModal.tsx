// Filament settings editor — the per-filament analogue of the printer
// panel's Machine tab. Opens on a bundled filament (by slug + name): the
// filament is edited in place, so it keeps its name ("Generic PLA"). Lists
// the Filament-bucket options grouped by libslic3r category in a left nav,
// and persists each edit live to the filament's override profile
// (`user_filament_set_override`), which is created transparently on the
// first edit.
//
// Overrides are read from the (possibly absent) override profile and kept
// current from each mutation's return value; the base (pre-override) values
// come from `user_filament_resolved_config`.

import { useEffect, useMemo, useState } from "react";
import { ModalBackdrop, ModalCloseButton } from "../ui/Modal";
import { categorize } from "../settings/nav/categories";
import { useFilamentOptions } from "../settings/resolve";
import { FilamentSettingsSection } from "./FilamentSettingsSection";
import {
  getUserFilament,
  setFilamentOverride,
  resolvedFilamentConfig,
} from "./userFilament";

export interface FilamentSettingsModalProps {
  /** Bundled filament slug to edit (also its identity). */
  base: string;
  /** Display name shown in the header (the bundled name; editing is in place). */
  name: string;
  onClose: () => void;
}

export function FilamentSettingsModal({
  base,
  name,
  onClose,
}: FilamentSettingsModalProps): React.JSX.Element {
  const { options } = useFilamentOptions();
  const [overrides, setOverrides] = useState<Record<string, string>>({});
  const [resolved, setResolved] = useState<Record<string, string>>({});
  const [active, setActive] = useState<string>("");
  const [error, setError] = useState<string | null>(null);

  // Load any existing overrides + the base values once. Overrides stay
  // current from each set/clear's return value below.
  useEffect(() => {
    let cancelled = false;
    getUserFilament(base)
      .then((f) => !cancelled && setOverrides(f?.overrides ?? {}))
      .catch((e) => setError(String(e)));
    resolvedFilamentConfig(base)
      .then((r) => !cancelled && setResolved(r))
      .catch((e) => setError(String(e)));
    return () => {
      cancelled = true;
    };
  }, [base]);

  const groups = useMemo(() => categorize(options), [options]);
  const navActive = groups.some((g) => g.id === active)
    ? active
    : (groups[0]?.id ?? "");

  const onSet = (key: string, value: string): void => {
    setFilamentOverride(base, key, value)
      .then((f) => setOverrides(f.overrides))
      .catch((e) => setError(String(e)));
  };
  const onClear = (key: string): void => {
    setFilamentOverride(base, key, null)
      .then((f) => setOverrides(f.overrides))
      .catch((e) => setError(String(e)));
  };

  return (
    <ModalBackdrop
      onDismiss={onClose}
      cardClassName="printer-settings-modal"
      backdropClassName="fsm-backdrop"
      ariaLabelledBy="fsm-title"
    >
      <header className="psm-header">
        <div className="psm-header-mark" data-brand="filament">
          <span>🧵</span>
        </div>
        <div className="psm-header-text">
          <h2 id="fsm-title">{name}</h2>
          <p>Filament settings</p>
        </div>
        <ModalCloseButton onClick={onClose} />
      </header>

      {error && <div className="fsm-error">{error}</div>}

      <div className="psm-body">
        <nav className="psm-nav" aria-label="Filament setting categories">
          {groups.map((g) => (
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

        <section className="psm-content">
          {groups.map((g) =>
            navActive === g.id ? (
              <FilamentSettingsSection
                key={g.id}
                settings={g.settings}
                overrides={overrides}
                resolved={resolved}
                onSet={onSet}
                onClear={onClear}
              />
            ) : null,
          )}
        </section>
      </div>
    </ModalBackdrop>
  );
}
