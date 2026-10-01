// Copies the packages Tauri buries in target/release/bundle/ into
// windows/release/, under the names they ship under. Used by `npm run pack` and
// by the release workflows, so both produce exactly the same file names.

import { readFileSync, mkdirSync, copyFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const bundleDir = join(root, "target", "release", "bundle");
const outDir = join(root, "release");

const { version } = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"));

// Newest wins, in case an older build is still lying around.
const newest = (dir, keep) => {
  try {
    return readdirSync(dir)
      .filter(keep)
      .map((f) => join(dir, f))
      .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)[0];
  } catch {
    return undefined;
  }
};

/** [source, shipping name] pairs to copy into release/. */
const ship = [];

if (process.platform === "win32") {
  const installer = newest(join(bundleDir, "nsis"), (f) => f.endsWith("-setup.exe"));
  if (!installer) {
    console.error(`No installer in ${join(bundleDir, "nsis")} — run \`npm run tauri build\` first.`);
    process.exit(1);
  }
  ship.push(
    [installer, `Atlas-Windows-${version}-setup.exe`],
    [installer, "Atlas-Windows-setup.exe"],
  );
} else if (process.platform === "linux") {
  const deb = newest(join(bundleDir, "deb"), (f) => f.endsWith(".deb"));
  if (!deb) {
    console.error(`No .deb in ${join(bundleDir, "deb")} — run \`npm run tauri build\` first.`);
    process.exit(1);
  }
  const appimage = newest(join(bundleDir, "appimage"), (f) => f.endsWith(".AppImage"));
  ship.push([deb, `Atlas-Linux-${version}.deb`], [deb, "Atlas-Linux.deb"]);
  if (appimage) {
    ship.push([appimage, `Atlas-Linux-${version}.AppImage`], [appimage, "Atlas-Linux.AppImage"]);
  }
} else {
  console.error(`Packing is not set up for ${process.platform} yet.`);
  process.exit(1);
}

mkdirSync(outDir, { recursive: true });
for (const [src, name] of ship) {
  copyFileSync(src, join(outDir, name));
  const mb = (statSync(join(outDir, name)).size / 1024 / 1024).toFixed(2);
  console.log(`  ${join(outDir, name)}  (${mb} MB)`);
}
