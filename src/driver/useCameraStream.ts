// useCameraStream — drive a live printer camera while a panel is visible.
//
// Lifecycle is frontend-driven (per the device-panel design): the stream
// opens when `active` goes true and tears down when it goes false or the
// component unmounts, so the printer's camera link is only held while the
// user is actually looking. The backend pushes raw JPEG frames over a
// Tauri Channel (delivered as ArrayBuffers); we wrap each in an object URL
// and swap it into `frameUrl`, revoking the previous so blobs don't leak.

import { useEffect, useRef, useState } from "react";
import { Channel } from "@tauri-apps/api/core";
import { cameraStart, cameraStop } from "./invokes";
import type { DriverConfig } from "./types";

export interface CameraStream {
  /** Object URL of the latest frame, or null before the first frame. */
  frameUrl: string | null;
  /** True once at least one frame has arrived (vs. still connecting). */
  live: boolean;
  /** Non-null if the stream couldn't start (e.g. unsupported backend). */
  error: string | null;
}

/** A primitive signature of the camera config, so the effect re-subscribes
 *  only when the target printer/credentials actually change — not on every
 *  render (the config object identity churns). Exported for testing. */
export function configSignature(config: DriverConfig | null): string | null {
  if (config == null) return null;
  if (config.kind === "Bambu") {
    return `Bambu|${config.data.host}|${config.data.access_code}`;
  }
  return `U1|${config.data.host}|${config.data.port}`;
}

export function useCameraStream(
  instanceId: string,
  config: DriverConfig | null,
  active: boolean,
): CameraStream {
  const [frameUrl, setFrameUrl] = useState<string | null>(null);
  const [live, setLive] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // The effect keys on a primitive signature; read the live config through
  // a ref so a host/code change re-runs the effect (signature changes)
  // while object-identity churn does not.
  const configRef = useRef(config);
  configRef.current = config;
  const sig = configSignature(config);

  useEffect(() => {
    const cfg = configRef.current;
    if (!active || cfg == null) return;

    let cancelled = false;
    let currentUrl: string | null = null;
    const channel = new Channel<ArrayBuffer>();
    channel.onmessage = (bytes) => {
      if (cancelled) return;
      const url = URL.createObjectURL(new Blob([bytes], { type: "image/jpeg" }));
      const previous = currentUrl;
      currentUrl = url;
      setFrameUrl(url);
      setLive(true);
      // Revoke the prior frame's URL only after swapping in the new one,
      // so the <img> never points at a revoked blob.
      if (previous) URL.revokeObjectURL(previous);
    };

    setError(null);
    setLive(false);
    cameraStart(instanceId, cfg, channel).catch((e) => {
      if (!cancelled) setError(String(e));
    });

    return () => {
      cancelled = true;
      // Tear down the printer link; idempotent on the backend.
      void cameraStop(instanceId);
      if (currentUrl) URL.revokeObjectURL(currentUrl);
      setFrameUrl(null);
      setLive(false);
    };
    // `config` is read via the ref; `sig` captures the parts that matter.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [instanceId, active, sig]);

  return { frameUrl, live, error };
}
