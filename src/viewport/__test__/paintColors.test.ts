import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { buildFaceColors } from "../paintColors";

describe("buildFaceColors", () => {
  it("colours all three vertices of each triangle by its state", () => {
    const states = new Uint8Array([1, 2]); // 2 triangles
    const palette: Record<number, number> = { 1: 0xff0000, 2: 0x00ff00 };
    const colors = buildFaceColors(states, (s) => palette[s]);
    expect(colors.length).toBe(2 * 9); // 2 tris × 3 verts × 3 channels

    const red = new THREE.Color().setHex(0xff0000);
    const green = new THREE.Color().setHex(0x00ff00);
    // Triangle 0 (state 1 → red): de-indexed vertices 0,1,2.
    for (let k = 0; k < 3; k++) {
      expect(colors[k * 3]).toBeCloseTo(red.r);
      expect(colors[k * 3 + 1]).toBeCloseTo(red.g);
      expect(colors[k * 3 + 2]).toBeCloseTo(red.b);
    }
    // Triangle 1 (state 2 → green): vertices 3,4,5 → byte offset 9.
    for (let k = 0; k < 3; k++) {
      expect(colors[9 + k * 3]).toBeCloseTo(green.r);
      expect(colors[9 + k * 3 + 1]).toBeCloseTo(green.g);
      expect(colors[9 + k * 3 + 2]).toBeCloseTo(green.b);
    }
  });

  it("passes each triangle's state to colorForState in order", () => {
    const seen: number[] = [];
    buildFaceColors(new Uint8Array([0, 5, 2]), (s) => {
      seen.push(s);
      return 0;
    });
    expect(seen).toEqual([0, 5, 2]);
  });

  it("returns an empty buffer for no triangles", () => {
    expect(buildFaceColors(new Uint8Array([]), () => 0).length).toBe(0);
  });
});
