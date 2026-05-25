// filament-catalog.jsx — A reasonably realistic catalog of filaments organized
// by brand → product line → color. Used by the filament picker dialog.
//
// Each color is { name, hex }. Each product captures sensible default temps
// (nozzleTemp / bedTemp) so they can seed a new filament's `print_temp` etc.
// Materials are a small enum: PLA, PETG, ABS, ASA, TPU, PC, PA, PVB, HIPS.

// Master color list — products reference subsets of this by name.
const COLOR_LIB = [
  { name: "Pure White",        hex: "#F2EFE7" },
  { name: "Cool White",        hex: "#E8EBEC" },
  { name: "Jade White",        hex: "#F1ECD9" },
  { name: "Ivory",             hex: "#EDE2C4" },
  { name: "Beige",             hex: "#D9CFB8" },
  { name: "Light Grey",        hex: "#BFC3C7" },
  { name: "Silver",            hex: "#A6ABB1" },
  { name: "Cool Grey",         hex: "#7A8794" },
  { name: "Charcoal",          hex: "#3A3E44" },
  { name: "Matte Black",       hex: "#1A1B1D" },
  { name: "Jet Black",         hex: "#0F1012" },
  { name: "Espresso",          hex: "#3B2A22" },
  { name: "Brown",             hex: "#7A4F2C" },
  { name: "Bronze",            hex: "#A37939" },
  { name: "Gold",              hex: "#C9A23E" },
  { name: "Sand",              hex: "#D6B97A" },
  { name: "Signal Red",        hex: "#C24B45" },
  { name: "Burgundy",          hex: "#6F2A2A" },
  { name: "Coral",             hex: "#E2735B" },
  { name: "Bright Orange",     hex: "#E07A2D" },
  { name: "Pumpkin",           hex: "#C45A1D" },
  { name: "Sunflower",         hex: "#E8C13D" },
  { name: "Lemon",             hex: "#F2D852" },
  { name: "Lime",              hex: "#A3CB47" },
  { name: "Grass Green",       hex: "#5BA34B" },
  { name: "Forest Green",      hex: "#2F6B3F" },
  { name: "Mint",              hex: "#7DC6A4" },
  { name: "Teal",              hex: "#2C8A8E" },
  { name: "Cyan",              hex: "#2BB6C2" },
  { name: "Sky Blue",          hex: "#6FA8D6" },
  { name: "Cobalt",            hex: "#2D55A6" },
  { name: "Navy",              hex: "#1E2F55" },
  { name: "Indigo",            hex: "#3A2E80" },
  { name: "Violet",            hex: "#7A5AE0" },
  { name: "Magenta",           hex: "#C13E94" },
  { name: "Hot Pink",          hex: "#E2569D" },
  { name: "Pastel Pink",       hex: "#F0B6CB" },
  { name: "Transparent",       hex: "#E8EEF0", translucent: true },
  { name: "Translucent Blue",  hex: "#A8C8E8", translucent: true },
  { name: "Translucent Red",   hex: "#E8A4A0", translucent: true },
  { name: "Natural",           hex: "#E8DFC6" },
];

const C = (...names) => names.map(n => {
  const c = COLOR_LIB.find(x => x.name === n);
  if (!c) console.warn("Missing color in catalog:", n);
  return c;
}).filter(Boolean);

// A broad palette many "consumer" products carry
const BROAD = ["Pure White", "Cool White", "Light Grey", "Cool Grey", "Charcoal",
               "Matte Black", "Jet Black", "Signal Red", "Burgundy", "Bright Orange",
               "Sunflower", "Lime", "Forest Green", "Mint", "Cyan",
               "Sky Blue", "Cobalt", "Navy", "Violet", "Magenta",
               "Hot Pink", "Pastel Pink", "Brown", "Beige", "Gold"];

const FILAMENT_CATALOG = [
  {
    brand: "Generic",
    short: "GEN",
    note: "Open baseline profiles — no vendor calibration",
    products: [
      { name: "PLA",      material: "PLA",  nozzleTemp: 215, bedTemp: 60,  colors: C(...BROAD) },
      { name: "PETG",     material: "PETG", nozzleTemp: 240, bedTemp: 80,  colors: C(...BROAD, "Transparent") },
      { name: "ABS",      material: "ABS",  nozzleTemp: 245, bedTemp: 100, colors: C("Pure White","Matte Black","Charcoal","Cool Grey","Signal Red","Cobalt","Bright Orange") },
      { name: "ASA",      material: "ASA",  nozzleTemp: 250, bedTemp: 100, colors: C("Pure White","Matte Black","Cool Grey","Charcoal","Signal Red","Beige") },
      { name: "TPU 95A",  material: "TPU",  nozzleTemp: 225, bedTemp: 50,  colors: C("Pure White","Matte Black","Cyan","Signal Red","Sunflower","Forest Green") },
      { name: "HIPS",     material: "HIPS", nozzleTemp: 230, bedTemp: 95,  colors: C("Pure White","Matte Black","Natural") },
    ],
  },
  {
    brand: "Bambu Lab",
    short: "BL",
    note: "First-party vendor profiles — AMS-compatible",
    products: [
      { name: "PLA Basic",        material: "PLA",  nozzleTemp: 220, bedTemp: 55, colors: C("Jade White","Pure White","Matte Black","Charcoal","Cool Grey","Signal Red","Burgundy","Bright Orange","Sunflower","Lime","Forest Green","Mint","Cyan","Cobalt","Navy","Violet","Magenta","Hot Pink","Gold","Bronze","Sand") },
      { name: "PLA Matte",        material: "PLA",  nozzleTemp: 220, bedTemp: 55, colors: C("Ivory","Matte Black","Cool Grey","Signal Red","Coral","Pumpkin","Lemon","Mint","Sky Blue","Indigo","Hot Pink","Pastel Pink","Espresso") },
      { name: "PLA Silk",         material: "PLA",  nozzleTemp: 230, bedTemp: 55, colors: C("Pure White","Gold","Bronze","Signal Red","Forest Green","Sky Blue","Violet","Magenta") },
      { name: "PLA-CF",           material: "PLA",  nozzleTemp: 240, bedTemp: 65, colors: C("Matte Black","Charcoal","Cool Grey") },
      { name: "PETG HF",          material: "PETG", nozzleTemp: 245, bedTemp: 70, colors: C("Pure White","Matte Black","Cool Grey","Signal Red","Bright Orange","Sunflower","Forest Green","Cyan","Cobalt","Magenta") },
      { name: "PETG Translucent", material: "PETG", nozzleTemp: 245, bedTemp: 70, colors: C("Transparent","Translucent Blue","Translucent Red") },
      { name: "PETG-CF",          material: "PETG", nozzleTemp: 260, bedTemp: 80, colors: C("Matte Black","Charcoal") },
      { name: "ABS",              material: "ABS",  nozzleTemp: 270, bedTemp: 100, colors: C("Pure White","Matte Black","Cool Grey","Signal Red","Cobalt") },
      { name: "ASA",              material: "ASA",  nozzleTemp: 270, bedTemp: 100, colors: C("Pure White","Matte Black","Cool Grey","Charcoal","Signal Red") },
      { name: "PA-CF",            material: "PA",   nozzleTemp: 290, bedTemp: 100, colors: C("Matte Black") },
      { name: "PC FR",            material: "PC",   nozzleTemp: 280, bedTemp: 100, colors: C("Pure White","Matte Black","Transparent") },
      { name: "Support for PLA",  material: "PLA",  nozzleTemp: 220, bedTemp: 55, colors: C("Natural") },
    ],
  },
  {
    brand: "Polymaker",
    short: "PM",
    note: "Engineering-grade lineup",
    products: [
      { name: "PolyTerra PLA",  material: "PLA",  nozzleTemp: 215, bedTemp: 60, colors: C("Ivory","Cool White","Matte Black","Charcoal","Cool Grey","Signal Red","Coral","Pumpkin","Sunflower","Mint","Forest Green","Sky Blue","Cobalt","Violet","Magenta","Pastel Pink","Espresso","Beige") },
      { name: "PolyLite PLA",   material: "PLA",  nozzleTemp: 215, bedTemp: 60, colors: C(...BROAD) },
      { name: "PolyLite PETG",  material: "PETG", nozzleTemp: 240, bedTemp: 80, colors: C("Pure White","Matte Black","Cool Grey","Signal Red","Sunflower","Forest Green","Cyan","Cobalt","Transparent") },
      { name: "PolyMax PC",     material: "PC",   nozzleTemp: 270, bedTemp: 100, colors: C("Pure White","Matte Black","Transparent") },
      { name: "PolyMide CoPA",  material: "PA",   nozzleTemp: 280, bedTemp: 90,  colors: C("Natural","Matte Black") },
      { name: "PolyFlex TPU95", material: "TPU",  nozzleTemp: 225, bedTemp: 50,  colors: C("Pure White","Matte Black","Signal Red","Cyan","Sunflower") },
    ],
  },
  {
    brand: "Prusament",
    short: "PRU",
    note: "Prusa Research's in-house spec",
    products: [
      { name: "PLA",  material: "PLA",  nozzleTemp: 215, bedTemp: 60, colors: C("Pure White","Matte Black","Charcoal","Cool Grey","Silver","Signal Red","Burgundy","Bright Orange","Sunflower","Lime","Forest Green","Mint","Cyan","Sky Blue","Cobalt","Navy","Violet","Magenta","Gold") },
      { name: "PETG", material: "PETG", nozzleTemp: 240, bedTemp: 80, colors: C("Pure White","Matte Black","Cool Grey","Signal Red","Sunflower","Forest Green","Cyan","Cobalt","Transparent","Translucent Blue","Translucent Red") },
      { name: "ASA",  material: "ASA",  nozzleTemp: 260, bedTemp: 100, colors: C("Pure White","Matte Black","Charcoal","Cool Grey","Signal Red") },
      { name: "PVB",  material: "PVB",  nozzleTemp: 215, bedTemp: 75, colors: C("Transparent","Pure White","Matte Black") },
    ],
  },
  {
    brand: "eSUN",
    short: "ES",
    note: "Budget-friendly, broad availability",
    products: [
      { name: "PLA+",   material: "PLA",  nozzleTemp: 215, bedTemp: 60, colors: C(...BROAD) },
      { name: "PETG",   material: "PETG", nozzleTemp: 245, bedTemp: 80, colors: C("Pure White","Matte Black","Cool Grey","Signal Red","Sunflower","Forest Green","Cobalt","Transparent") },
      { name: "ABS+",   material: "ABS",  nozzleTemp: 240, bedTemp: 100, colors: C("Pure White","Matte Black","Cool Grey","Signal Red","Cobalt") },
      { name: "eSilk",  material: "PLA",  nozzleTemp: 220, bedTemp: 60, colors: C("Gold","Bronze","Signal Red","Forest Green","Sky Blue","Magenta") },
    ],
  },
  {
    brand: "Overture",
    short: "OV",
    note: "Matte PLA specialist",
    products: [
      { name: "PLA Matte", material: "PLA",  nozzleTemp: 220, bedTemp: 60, colors: C("Ivory","Matte Black","Cool Grey","Charcoal","Signal Red","Coral","Pumpkin","Sunflower","Mint","Forest Green","Sky Blue","Indigo","Hot Pink","Pastel Pink","Beige","Espresso") },
      { name: "PETG",      material: "PETG", nozzleTemp: 240, bedTemp: 80, colors: C("Pure White","Matte Black","Cool Grey","Signal Red","Sunflower","Forest Green","Cobalt","Transparent") },
    ],
  },
  {
    brand: "SUNLU",
    short: "SU",
    note: "Value tier",
    products: [
      { name: "PLA Meta",  material: "PLA",  nozzleTemp: 200, bedTemp: 50, colors: C(...BROAD) },
      { name: "PLA Silk",  material: "PLA",  nozzleTemp: 215, bedTemp: 55, colors: C("Gold","Bronze","Signal Red","Cyan","Magenta","Hot Pink") },
      { name: "PETG",      material: "PETG", nozzleTemp: 240, bedTemp: 75, colors: C("Pure White","Matte Black","Cool Grey","Signal Red","Transparent") },
    ],
  },
  {
    brand: "Jupyter",
    short: "JP",
    note: "Composite & engineering focus",
    products: [
      { name: "PETG-CF",  material: "PETG", nozzleTemp: 250, bedTemp: 80,  colors: C("Charcoal","Matte Black") },
      { name: "PETG-GF",  material: "PETG", nozzleTemp: 250, bedTemp: 80,  colors: C("Charcoal","Matte Black","Natural") },
      { name: "PLA Pro",  material: "PLA",  nozzleTemp: 220, bedTemp: 60,  colors: C("Pure White","Matte Black","Cool Grey","Signal Red","Forest Green","Cobalt","Bronze") },
      { name: "PA12-CF",  material: "PA",   nozzleTemp: 295, bedTemp: 100, colors: C("Matte Black") },
      { name: "PC-CF",    material: "PC",   nozzleTemp: 290, bedTemp: 110, colors: C("Matte Black") },
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
window.filamentSlug = filamentSlug;
