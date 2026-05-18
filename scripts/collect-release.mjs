import { copyFileSync, existsSync, mkdirSync, readdirSync, rmSync, statSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const targetRelease = join(repoRoot, "src-tauri", "target", "release");
const releaseDir = join(repoRoot, "release");

const sources = [
  join(targetRelease, "tdl-desktop.exe"),
  ...filesIn(join(targetRelease, "bundle", "nsis"), ".exe"),
  ...filesIn(join(targetRelease, "bundle", "msi"), ".msi"),
];

if (sources.length === 0) {
  throw new Error("No release artifacts found. Run `npm run tauri:build` first.");
}

mkdirSync(releaseDir, { recursive: true });
for (const file of readdirSync(releaseDir)) {
  if (file.toLowerCase().endsWith(".exe") || file.toLowerCase().endsWith(".msi")) {
    rmSync(join(releaseDir, file), { force: true, maxRetries: 5, retryDelay: 200 });
  }
}

for (const source of sources) {
  if (!existsSync(source)) {
    throw new Error(`Missing release artifact: ${source}`);
  }
  copyFileSync(source, join(releaseDir, basename(source)));
}

console.log(`Release artifacts copied to ${releaseDir}`);
for (const file of readdirSync(releaseDir)) {
  const path = join(releaseDir, file);
  console.log(`- ${file} (${statSync(path).size} bytes)`);
}

function filesIn(directory, extension) {
  if (!existsSync(directory)) {
    return [];
  }

  return readdirSync(directory)
    .filter((file) => file.toLowerCase().endsWith(extension))
    .map((file) => join(directory, file));
}
