// Curated color palette shared by every product in the filament
// picker modal. Mirrors `STANDARD_PALETTE` from `docs/dev/design/
// filament-catalog.jsx`.
//
// Vendor profiles (OrcaSlicer/BBS) don't model per-color SKUs — color
// lives on the slot, not the profile. The picker therefore uses one
// palette across all products; picking a color writes
// `SlotBinding.color`. The Custom swatch lets the user pick any hex
// when nothing in the palette matches the spool.
//
// Entries are grouped loosely by hue family so the grid reads as a
// rainbow when rendered in order.

export interface PaletteColor {
  name: string;
  hex: string;
  translucent?: boolean;
}

export const FILAMENT_COLOR_PALETTE: readonly PaletteColor[] = [
  // Whites & naturals
  { name: "Pure White", hex: "#F4F1EA" },
  { name: "Cool White", hex: "#E8EBEC" },
  { name: "Jade White", hex: "#F1ECD9" },
  { name: "Cream", hex: "#F2E6C9" },
  { name: "Bone", hex: "#E8DCC6" },
  { name: "Ivory", hex: "#EDE2C4" },
  { name: "Natural", hex: "#E8DFC6" },
  { name: "Beige", hex: "#D9CFB8" },

  // Greys
  { name: "Light Grey", hex: "#BFC3C7" },
  { name: "Silver", hex: "#A6ABB1" },
  { name: "Cool Grey", hex: "#7A8794" },
  { name: "Slate", hex: "#5B6772" },
  { name: "Gunmetal", hex: "#3E464F" },
  { name: "Charcoal", hex: "#3A3E44" },

  // Blacks
  { name: "Matte Black", hex: "#1A1B1D" },
  { name: "Jet Black", hex: "#0F1012" },

  // Browns & earths
  { name: "Khaki", hex: "#A39369" },
  { name: "Sand", hex: "#D6B97A" },
  { name: "Brown", hex: "#7A4F2C" },
  { name: "Espresso", hex: "#3B2A22" },

  // Metallics
  { name: "Rose Gold", hex: "#C58C7A" },
  { name: "Copper", hex: "#B86A3C" },
  { name: "Bronze", hex: "#A37939" },
  { name: "Gold", hex: "#C9A23E" },

  // Reds
  { name: "Salmon", hex: "#E89788" },
  { name: "Coral", hex: "#E2735B" },
  { name: "Signal Red", hex: "#C24B45" },
  { name: "Crimson", hex: "#9E2A2A" },
  { name: "Burgundy", hex: "#6F2A2A" },
  { name: "Wine", hex: "#4A1F2A" },

  // Pinks
  { name: "Pastel Pink", hex: "#F0B6CB" },
  { name: "Rose", hex: "#D87099" },
  { name: "Hot Pink", hex: "#E2569D" },
  { name: "Magenta", hex: "#C13E94" },

  // Oranges
  { name: "Apricot", hex: "#EEA56A" },
  { name: "Bright Orange", hex: "#E07A2D" },
  { name: "Pumpkin", hex: "#C45A1D" },

  // Yellows
  { name: "Lemon", hex: "#F2D852" },
  { name: "Sunflower", hex: "#E8C13D" },
  { name: "Mustard", hex: "#B89028" },

  // Greens
  { name: "Lime", hex: "#A3CB47" },
  { name: "Yellow Green", hex: "#8CB23A" },
  { name: "Olive", hex: "#6E7A2C" },
  { name: "Grass Green", hex: "#5BA34B" },
  { name: "Sage", hex: "#9DB28C" },
  { name: "Mint", hex: "#7DC6A4" },
  { name: "Emerald", hex: "#2F8F5C" },
  { name: "Forest Green", hex: "#2F6B3F" },

  // Cyans & teals
  { name: "Aqua", hex: "#5FCFD0" },
  { name: "Teal", hex: "#2C8A8E" },
  { name: "Cyan", hex: "#2BB6C2" },

  // Blues
  { name: "Sky Blue", hex: "#6FA8D6" },
  { name: "Steel Blue", hex: "#4A7CA8" },
  { name: "Royal Blue", hex: "#2E58C2" },
  { name: "Cobalt", hex: "#2D55A6" },
  { name: "Navy", hex: "#1E2F55" },

  // Purples & violets
  { name: "Periwinkle", hex: "#9AA3E0" },
  { name: "Lavender", hex: "#B79EE0" },
  { name: "Lilac", hex: "#9C7BC2" },
  { name: "Violet", hex: "#7A5AE0" },
  { name: "Indigo", hex: "#3A2E80" },
  { name: "Plum", hex: "#5A2E66" },

  // Translucents
  { name: "Transparent", hex: "#E8EEF0", translucent: true },
  { name: "Translucent Blue", hex: "#A8C8E8", translucent: true },
  { name: "Translucent Red", hex: "#E8A4A0", translucent: true },
  { name: "Translucent Green", hex: "#B5DDB8", translucent: true },
  { name: "Translucent Yellow", hex: "#EFE3A8", translucent: true },
  { name: "Translucent Purple", hex: "#C4AEDC", translucent: true },
];

/** Look up a palette entry by hex (case-insensitive). Returns null
 *  when the color isn't one of the curated entries — picker then
 *  seeds the Custom swatch with that color. */
export function paletteEntryForHex(
  hex: string | null,
): PaletteColor | null {
  if (!hex) return null;
  const needle = hex.toLowerCase();
  return (
    FILAMENT_COLOR_PALETTE.find((c) => c.hex.toLowerCase() === needle) ?? null
  );
}
