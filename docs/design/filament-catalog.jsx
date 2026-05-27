// filament-catalog.jsx — A reasonably realistic catalog of filaments organized
// by brand → product line. Used by the filament picker dialog.
//
// Each product captures sensible default temps (nozzleTemp / bedTemp) so they
// can seed a new filament's `print_temp` etc. Materials are a small enum:
// PLA, PETG, ABS, ASA, TPU, PC, PA, PVB, HIPS.
//
// Color is decoupled from the product — every product can be ordered in any
// color from STANDARD_PALETTE below. Real-world product-specific color
// availability isn't modelled here; pick any color, get any color.

// ───────── Standard color palette ─────────
// Extensive shared palette used for every product across every brand. Grouped
// loosely by hue family so the picker grid reads as a rainbow when rendered
// in order.
const STANDARD_PALETTE = [
  // Whites & naturals
  { name: "Pure White",        hex: "#F4F1EA" },
  { name: "Cool White",        hex: "#E8EBEC" },
  { name: "Jade White",        hex: "#F1ECD9" },
  { name: "Cream",             hex: "#F2E6C9" },
  { name: "Bone",              hex: "#E8DCC6" },
  { name: "Ivory",             hex: "#EDE2C4" },
  { name: "Natural",           hex: "#E8DFC6" },
  { name: "Beige",             hex: "#D9CFB8" },

  // Greys
  { name: "Light Grey",        hex: "#BFC3C7" },
  { name: "Silver",             hex: "#A6ABB1" },
  { name: "Cool Grey",         hex: "#7A8794" },
  { name: "Slate",             hex: "#5B6772" },
  { name: "Gunmetal",          hex: "#3E464F" },
  { name: "Charcoal",          hex: "#3A3E44" },

  // Blacks
  { name: "Matte Black",       hex: "#1A1B1D" },
  { name: "Jet Black",         hex: "#0F1012" },

  // Browns & earths
  { name: "Khaki",             hex: "#A39369" },
  { name: "Sand",              hex: "#D6B97A" },
  { name: "Brown",             hex: "#7A4F2C" },
  { name: "Espresso",          hex: "#3B2A22" },

  // Metallics
  { name: "Rose Gold",         hex: "#C58C7A" },
  { name: "Copper",            hex: "#B86A3C" },
  { name: "Bronze",            hex: "#A37939" },
  { name: "Gold",              hex: "#C9A23E" },

  // Reds
  { name: "Salmon",            hex: "#E89788" },
  { name: "Coral",             hex: "#E2735B" },
  { name: "Signal Red",        hex: "#C24B45" },
  { name: "Crimson",           hex: "#9E2A2A" },
  { name: "Burgundy",          hex: "#6F2A2A" },
  { name: "Wine",              hex: "#4A1F2A" },

  // Pinks
  { name: "Pastel Pink",       hex: "#F0B6CB" },
  { name: "Rose",              hex: "#D87099" },
  { name: "Hot Pink",          hex: "#E2569D" },
  { name: "Magenta",           hex: "#C13E94" },

  // Oranges
  { name: "Apricot",           hex: "#EEA56A" },
  { name: "Bright Orange",     hex: "#E07A2D" },
  { name: "Pumpkin",           hex: "#C45A1D" },

  // Yellows
  { name: "Lemon",             hex: "#F2D852" },
  { name: "Sunflower",         hex: "#E8C13D" },
  { name: "Mustard",           hex: "#B89028" },

  // Greens
  { name: "Lime",              hex: "#A3CB47" },
  { name: "Yellow Green",      hex: "#8CB23A" },
  { name: "Olive",             hex: "#6E7A2C" },
  { name: "Grass Green",       hex: "#5BA34B" },
  { name: "Sage",              hex: "#9DB28C" },
  { name: "Mint",              hex: "#7DC6A4" },
  { name: "Emerald",           hex: "#2F8F5C" },
  { name: "Forest Green",      hex: "#2F6B3F" },

  // Cyans & teals
  { name: "Aqua",              hex: "#5FCFD0" },
  { name: "Teal",              hex: "#2C8A8E" },
  { name: "Cyan",              hex: "#2BB6C2" },

  // Blues
  { name: "Sky Blue",          hex: "#6FA8D6" },
  { name: "Steel Blue",        hex: "#4A7CA8" },
  { name: "Royal Blue",        hex: "#2E58C2" },
  { name: "Cobalt",            hex: "#2D55A6" },
  { name: "Navy",              hex: "#1E2F55" },

  // Purples & violets
  { name: "Periwinkle",        hex: "#9AA3E0" },
  { name: "Lavender",          hex: "#B79EE0" },
  { name: "Lilac",             hex: "#9C7BC2" },
  { name: "Violet",            hex: "#7A5AE0" },
  { name: "Indigo",            hex: "#3A2E80" },
  { name: "Plum",              hex: "#5A2E66" },

  // Translucents
  { name: "Transparent",         hex: "#E8EEF0", translucent: true },
  { name: "Translucent Blue",    hex: "#A8C8E8", translucent: true },
  { name: "Translucent Red",     hex: "#E8A4A0", translucent: true },
  { name: "Translucent Green",   hex: "#B5DDB8", translucent: true },
  { name: "Translucent Yellow",  hex: "#EFE3A8", translucent: true },
  { name: "Translucent Purple",  hex: "#C4AEDC", translucent: true },
];

// ───────── Brand & product catalog ─────────
// Products no longer carry color lists — every product can be ordered in any
// color from STANDARD_PALETTE.
const FILAMENT_CATALOG = [
  {
    brand: "Generic",
    short: "GEN",
    note: "Open baseline profiles — no vendor calibration",
    products: [
      { name: "PLA",      material: "PLA",  nozzleTemp: 215, bedTemp: 60  },
      { name: "PETG",     material: "PETG", nozzleTemp: 240, bedTemp: 80  },
      { name: "ABS",      material: "ABS",  nozzleTemp: 245, bedTemp: 100 },
      { name: "ASA",      material: "ASA",  nozzleTemp: 250, bedTemp: 100 },
      { name: "TPU 95A",  material: "TPU",  nozzleTemp: 225, bedTemp: 50  },
      { name: "HIPS",     material: "HIPS", nozzleTemp: 230, bedTemp: 95  },
    ],
  },
  {
    brand: "Bambu Lab",
    short: "BL",
    note: "First-party vendor profiles — AMS-compatible",
    products: [
      { name: "PLA Basic",        material: "PLA",  nozzleTemp: 220, bedTemp: 55  },
      { name: "PLA Matte",        material: "PLA",  nozzleTemp: 220, bedTemp: 55  },
      { name: "PLA Silk",         material: "PLA",  nozzleTemp: 230, bedTemp: 55  },
      { name: "PLA-CF",           material: "PLA",  nozzleTemp: 240, bedTemp: 65  },
      { name: "PETG HF",          material: "PETG", nozzleTemp: 245, bedTemp: 70  },
      { name: "PETG Translucent", material: "PETG", nozzleTemp: 245, bedTemp: 70  },
      { name: "PETG-CF",          material: "PETG", nozzleTemp: 260, bedTemp: 80  },
      { name: "ABS",              material: "ABS",  nozzleTemp: 270, bedTemp: 100 },
      { name: "ASA",              material: "ASA",  nozzleTemp: 270, bedTemp: 100 },
      { name: "PA-CF",            material: "PA",   nozzleTemp: 290, bedTemp: 100 },
      { name: "PC FR",            material: "PC",   nozzleTemp: 280, bedTemp: 100 },
      { name: "Support for PLA",  material: "PLA",  nozzleTemp: 220, bedTemp: 55  },
    ],
  },
  {
    brand: "Polymaker",
    short: "PM",
    note: "Engineering-grade lineup",
    products: [
      { name: "PolyTerra PLA",  material: "PLA",  nozzleTemp: 215, bedTemp: 60  },
      { name: "PolyLite PLA",   material: "PLA",  nozzleTemp: 215, bedTemp: 60  },
      { name: "PolyLite PETG",  material: "PETG", nozzleTemp: 240, bedTemp: 80  },
      { name: "PolyMax PC",     material: "PC",   nozzleTemp: 270, bedTemp: 100 },
      { name: "PolyMide CoPA",  material: "PA",   nozzleTemp: 280, bedTemp: 90  },
      { name: "PolyFlex TPU95", material: "TPU",  nozzleTemp: 225, bedTemp: 50  },
    ],
  },
  {
    brand: "Prusament",
    short: "PRU",
    note: "Prusa Research's in-house spec",
    products: [
      { name: "PLA",  material: "PLA",  nozzleTemp: 215, bedTemp: 60  },
      { name: "PETG", material: "PETG", nozzleTemp: 240, bedTemp: 80  },
      { name: "ASA",  material: "ASA",  nozzleTemp: 260, bedTemp: 100 },
      { name: "PVB",  material: "PVB",  nozzleTemp: 215, bedTemp: 75  },
    ],
  },
  {
    brand: "eSUN",
    short: "ES",
    note: "Budget-friendly, broad availability",
    products: [
      { name: "PLA+",   material: "PLA",  nozzleTemp: 215, bedTemp: 60  },
      { name: "PETG",   material: "PETG", nozzleTemp: 245, bedTemp: 80  },
      { name: "ABS+",   material: "ABS",  nozzleTemp: 240, bedTemp: 100 },
      { name: "eSilk",  material: "PLA",  nozzleTemp: 220, bedTemp: 60  },
    ],
  },
  {
    brand: "Overture",
    short: "OV",
    note: "Matte PLA specialist",
    products: [
      { name: "PLA Matte", material: "PLA",  nozzleTemp: 220, bedTemp: 60 },
      { name: "PETG",      material: "PETG", nozzleTemp: 240, bedTemp: 80 },
    ],
  },
  {
    brand: "SUNLU",
    short: "SU",
    note: "Value tier",
    products: [
      { name: "PLA Meta",  material: "PLA",  nozzleTemp: 200, bedTemp: 50 },
      { name: "PLA Silk",  material: "PLA",  nozzleTemp: 215, bedTemp: 55 },
      { name: "PETG",      material: "PETG", nozzleTemp: 240, bedTemp: 75 },
    ],
  },
  {
    brand: "Jupyter",
    short: "JP",
    note: "Composite & engineering focus",
    products: [
      { name: "PETG-CF",  material: "PETG", nozzleTemp: 250, bedTemp: 80  },
      { name: "PETG-GF",  material: "PETG", nozzleTemp: 250, bedTemp: 80  },
      { name: "PLA Pro",  material: "PLA",  nozzleTemp: 220, bedTemp: 60  },
      { name: "PA12-CF",  material: "PA",   nozzleTemp: 295, bedTemp: 100 },
      { name: "PC-CF",    material: "PC",   nozzleTemp: 290, bedTemp: 110 },
    ],
  },
];

// Flattened helper: all materials present anywhere in catalog (for the filter chips).
const CATALOG_MATERIALS = Array.from(
  new Set(FILAMENT_CATALOG.flatMap(b => b.products.map(p => p.material)))
);

// Build a small slug for filament IDs derived from brand/product/color.
function filamentSlug(brand, product, colorName) {
  return [brand, product, colorName]
    .join("-")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_|_$/g, "");
}

window.FILAMENT_CATALOG = FILAMENT_CATALOG;
window.CATALOG_MATERIALS = CATALOG_MATERIALS;
window.STANDARD_PALETTE = STANDARD_PALETTE;
window.filamentSlug = filamentSlug;
