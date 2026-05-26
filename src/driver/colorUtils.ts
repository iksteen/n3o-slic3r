// Color helpers shared across driver-status strips.
//
// Drivers report filament color as `RRGGBBAA` hex without `#` (the
// Bambu MQTT shape; mirrored by the Snapmaker print_task_config
// fields). Strips render the value as a CSS background — this
// helper normalizes the wire form to a CSS color string and drops
// the alpha when fully opaque so the rendered hex stays short.

/** RRGGBBAA wire-hex → CSS color. Drops alpha if it's `FF`; keeps
 * full 8-digit form otherwise so transparency renders correctly. */
export function cssColorFromHex(wire: string): string {
  if (wire.length === 8) {
    const alpha = wire.slice(6, 8).toLowerCase();
    if (alpha === "ff") {
      return `#${wire.slice(0, 6)}`;
    }
  }
  return `#${wire}`;
}
