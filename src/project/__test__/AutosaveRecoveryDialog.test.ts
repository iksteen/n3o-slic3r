// Autosave-recovery dialog — formatter tests.
//
// The full dialog renders DOM (modal, action buttons, async
// invoke flow) which our vitest setup doesn't have a DOM for.
// The pure formatters are what's worth pinning here; they drive
// the per-row "2h ago / 12.3 KB" surface.

import { describe, expect, it } from "vitest";
import {
  formatBytes,
  formatRelative,
} from "../AutosaveRecoveryDialog";

describe("formatRelative", () => {
  const NOW_MS = 1_716_480_000_000; // arbitrary fixed "now"

  it("renders seconds bucket for very recent saves", () => {
    expect(formatRelative(1_716_479_995, NOW_MS)).toBe("5s ago");
  });

  it("renders minutes bucket", () => {
    expect(formatRelative(1_716_479_700, NOW_MS)).toBe("5m ago");
  });

  it("renders hours bucket", () => {
    expect(formatRelative(1_716_472_800, NOW_MS)).toBe("2h ago");
  });

  it("renders days bucket for old saves", () => {
    expect(formatRelative(1_716_307_200, NOW_MS)).toBe("2d ago");
  });

  it("clamps to 0s for a timestamp in the future (clock skew)", () => {
    // The autosave file might have a slightly-future mtime if the
    // wall clock jumped backward between save and read. Don't
    // render negative ages — they look like a bug.
    expect(formatRelative(1_716_481_000, NOW_MS)).toBe("0s ago");
  });
});

describe("formatBytes", () => {
  it("renders bytes directly under 1 KB", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1023)).toBe("1023 B");
  });

  it("renders kilobytes with one decimal place", () => {
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(102400)).toBe("100.0 KB");
  });

  it("renders megabytes with one decimal place", () => {
    expect(formatBytes(1024 * 1024)).toBe("1.0 MB");
    expect(formatBytes(5 * 1024 * 1024 + 512 * 1024)).toBe("5.5 MB");
  });
});
