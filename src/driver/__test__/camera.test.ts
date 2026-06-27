import { describe, expect, it } from "vitest";
import { configSignature } from "../useCameraStream";
import { cameraPlaceholder } from "../DeviceCamera";
import type { DriverConfig } from "../types";

const bambu = (host: string, accessCode: string): DriverConfig => ({
  kind: "Bambu",
  data: { host, access_code: accessCode },
});

describe("configSignature", () => {
  it("is null for no config (the inert / unconfigured case)", () => {
    expect(configSignature(null)).toBeNull();
  });

  it("changes when the host or access code changes", () => {
    const a = configSignature(bambu("192.168.1.50", "11111111"));
    expect(configSignature(bambu("192.168.1.51", "11111111"))).not.toBe(a);
    expect(configSignature(bambu("192.168.1.50", "22222222"))).not.toBe(a);
  });

  it("is stable across object identity for the same credentials", () => {
    expect(configSignature(bambu("192.168.1.50", "11111111"))).toBe(
      configSignature(bambu("192.168.1.50", "11111111")),
    );
  });

  it("distinguishes backends with the same host", () => {
    const u1: DriverConfig = { kind: "U1", data: { host: "192.168.1.50", port: 80 } };
    expect(configSignature(u1)).not.toBe(configSignature(bambu("192.168.1.50", "x")));
  });
});

describe("cameraPlaceholder", () => {
  it("reports unsupported backends first, regardless of online state", () => {
    const p = cameraPlaceholder({ supported: false, offline: false, error: null });
    expect(p.detail).toBe("Not available for this printer");
    expect(p.slashed).toBe(true);
  });

  it("shows offline before surfacing any error", () => {
    const p = cameraPlaceholder({ supported: true, offline: true, error: "boom" });
    expect(p.detail).toBe("Printer offline");
  });

  it("surfaces a start error when supported and online", () => {
    const p = cameraPlaceholder({ supported: true, offline: false, error: "TLS handshake failed" });
    expect(p.title).toBe("Camera unavailable");
    expect(p.detail).toBe("TLS handshake failed");
  });

  it("connecting state uses the un-slashed glyph and no detail", () => {
    const p = cameraPlaceholder({ supported: true, offline: false, error: null });
    expect(p.title).toBe("Connecting to camera…");
    expect(p.detail).toBeNull();
    expect(p.slashed).toBe(false);
  });
});
