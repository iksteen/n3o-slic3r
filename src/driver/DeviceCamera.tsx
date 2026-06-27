// Live printer camera panel + its placeholder logic for the Devices monitor.

import { useCameraStream } from "./useCameraStream";
import type { DriverConfig } from "./types";

/** The camera glyph. `slashed` draws the struck-through (disabled) form
 *  for the unavailable/offline states; the plain form reads as "live /
 *  connecting". */
function CameraIcon({ slashed }: { slashed: boolean }): React.JSX.Element {
  return (
    <svg width="32" height="32" viewBox="0 0 32 32" fill="none" opacity="0.4">
      <rect x="3" y="8" width="20" height="16" rx="2" stroke="currentColor" strokeWidth="1.5" />
      <path
        d="M23 13l6-3v12l-6-3z"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinejoin="round"
      />
      {slashed && <path d="M3 3l26 26" stroke="currentColor" strokeWidth="1.5" />}
    </svg>
  );
}

export interface CameraPlaceholder {
  title: string;
  detail: string | null;
  /** Draw the struck-through camera glyph (disabled look). */
  slashed: boolean;
}

/** Pick the non-live camera placeholder, most-specific case first. Pure so
 *  the state logic is unit-testable without a DOM. */
export function cameraPlaceholder(args: {
  /** Backend has a wired camera (Bambu today). */
  supported: boolean;
  offline: boolean;
  /** Stream start error, if any. */
  error: string | null;
}): CameraPlaceholder {
  if (!args.supported) {
    return { title: "Webcam", detail: "Not available for this printer", slashed: true };
  }
  if (args.offline) {
    return { title: "Webcam", detail: "Printer offline", slashed: true };
  }
  if (args.error) {
    return { title: "Camera unavailable", detail: args.error, slashed: true };
  }
  return { title: "Connecting to camera…", detail: null, slashed: false };
}

/** Live printer camera. The stream is opened only while this panel is
 *  mounted and the printer is online (the frontend owns the lifecycle —
 *  see `useCameraStream`); the backend pushes JPEG frames we render into an
 *  `<img>`. Non-camera backends (and offline printers) show a placeholder
 *  without opening any link. */
export function CameraPanel({
  instanceId,
  config,
  offline,
}: {
  instanceId: string;
  /** The instance's driver config, or null when unconfigured. */
  config: DriverConfig | null;
  offline: boolean;
}): React.JSX.Element {
  // Bambu LAN and Snapmaker U1 cameras are wired. An unpaired U1 still
  // counts as "supported" — camera_start rejects with pairing guidance,
  // which surfaces as the error placeholder rather than a generic
  // "not available".
  const supported = config?.kind === "Bambu" || config?.kind === "U1";
  const active = supported && !offline;
  const { frameUrl, error } = useCameraStream(
    instanceId,
    active ? config : null,
    active,
  );

  if (active && frameUrl) {
    return (
      <div className="device-camera">
        <img className="device-camera-img" src={frameUrl} alt="Printer camera" />
      </div>
    );
  }

  const placeholder = cameraPlaceholder({ supported, offline, error });

  return (
    <div className="device-camera off">
      <div className="device-camera-frame">
        <div className="device-camera-off-msg">
          <CameraIcon slashed={placeholder.slashed} />
          <div>{placeholder.title}</div>
          {placeholder.detail && <div className="dim">{placeholder.detail}</div>}
        </div>
      </div>
    </div>
  );
}
