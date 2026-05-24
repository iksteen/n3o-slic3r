// PR-7a-7 credentials-dialog validation tests.
//
// Component rendering / submit lifecycle (React + Tauri) needs a
// jsdom + RTL setup we don't have. We pin the pure validator that
// gates submission so a regression in the access-code format
// check or host-presence check trips loudly.

import { describe, expect, it } from "vitest";
import { validateCredentials } from "../PrinterCredentialsDialog";

describe("validateCredentials", () => {
  it("accepts a complete + correctly-shaped credential", () => {
    expect(
      validateCredentials({
        host: "192.168.1.42",
        access_code: "12345678",
        serial: "01S00A123400000",
      }),
    ).toBeNull();
  });

  it("accepts a null serial (driver probes from peer cert)", () => {
    expect(
      validateCredentials({
        host: "192.168.1.42",
        access_code: "12345678",
        serial: null,
      }),
    ).toBeNull();
  });

  it("treats whitespace-only serial as null (passes validation)", () => {
    // The dialog trims serial before calling — but if a caller
    // hands an empty string we shouldn't reject it; that's
    // semantically "let the driver probe".
    expect(
      validateCredentials({
        host: "192.168.1.42",
        access_code: "12345678",
        serial: "   ",
      }),
    ).toBeNull();
  });

  it("rejects empty host", () => {
    expect(
      validateCredentials({
        host: "",
        access_code: "12345678",
        serial: null,
      }),
    ).toMatch(/Host/);
  });

  it("rejects whitespace-only host", () => {
    expect(
      validateCredentials({
        host: "   ",
        access_code: "12345678",
        serial: null,
      }),
    ).toMatch(/Host/);
  });

  it("rejects non-numeric access code", () => {
    expect(
      validateCredentials({
        host: "192.168.1.42",
        access_code: "abcdefgh",
        serial: null,
      }),
    ).toMatch(/8 digits/);
  });

  it("rejects access code shorter than 8 chars", () => {
    expect(
      validateCredentials({
        host: "192.168.1.42",
        access_code: "1234567",
        serial: null,
      }),
    ).toMatch(/8 digits/);
  });

  it("rejects access code longer than 8 chars", () => {
    expect(
      validateCredentials({
        host: "192.168.1.42",
        access_code: "123456789",
        serial: null,
      }),
    ).toMatch(/8 digits/);
  });
});
