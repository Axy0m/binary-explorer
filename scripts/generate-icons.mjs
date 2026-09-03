// Regenerates the Tauri app icon set from the Nybble brand mark.
//
//   npm run gen:icons
//
// The source of truth is assets/brand/nybble-icon.svg. This script only drives
// `tauri icon`, which rasterizes that SVG into every size Tauri bundles plus the
// Windows .ico and macOS .icns — so the icons always match the brand artwork
// rather than being drawn here.
//
// Tauri also emits iOS/Android icon sets. This app is desktop-only, so they are
// removed afterwards to keep the icons directory to what actually ships.
import { execFileSync } from "node:child_process";
import { existsSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const SOURCE = join(ROOT, "assets", "brand", "nybble-icon.svg");
const ICON_DIR = join(ROOT, "apps", "desktop", "src-tauri", "icons");

if (!existsSync(SOURCE)) {
  console.error(`Brand mark not found: ${SOURCE}`);
  process.exit(1);
}

console.log(`Generating icons from ${SOURCE}`);
execFileSync("npx", ["tauri", "icon", SOURCE, "-o", ICON_DIR], {
  cwd: ROOT,
  stdio: "inherit",
  shell: process.platform === "win32",
});

for (const dir of ["android", "ios"]) {
  const path = join(ICON_DIR, dir);
  if (existsSync(path)) {
    rmSync(path, { recursive: true, force: true });
    console.log(`  removed ${dir}/ (desktop-only app)`);
  }
}

console.log("Done.");
