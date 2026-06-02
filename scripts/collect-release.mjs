import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
} from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const targetRelease = join(repoRoot, "src-tauri", "target", "release");
const releaseDir = join(repoRoot, "release");
const packageJson = JSON.parse(readFileSync(join(repoRoot, "package.json"), "utf8"));

const productSlug = slugify(packageJson.name ?? "tdl-desktop");
const version = packageJson.version;
const platform = releasePlatform();
const artifactBase = `${productSlug}-v${version}-${platform}`;

const appExe = join(targetRelease, "tdl-desktop.exe");
const bundledTdl = join(targetRelease, "resources", "tdl.exe");
const msiFiles = filesIn(join(targetRelease, "bundle", "msi"), ".msi");
const msi = newestFile(msiFiles.filter((file) => basename(file).includes(version)));
const portableZip = join(releaseDir, `${artifactBase}-portable.zip`);
const installerMsi = join(releaseDir, `${artifactBase}-installer.msi`);
const stagingDir = join(releaseDir, ".portable-staging");

if (!version) {
  throw new Error("Missing package version in package.json");
}
assertExists(appExe, "Missing release executable. Run `npm run tauri:build` first.");
assertExists(bundledTdl, "Missing bundled tdl.exe. Run `npm run fetch:tdl` before packaging.");
if (!msi) {
  const available = msiFiles.map((file) => basename(file)).join(", ") || "none";
  throw new Error(`No MSI artifact found for version ${version}. Available: ${available}`);
}

mkdirSync(releaseDir, { recursive: true });
cleanReleaseDir();
rmSync(stagingDir, { recursive: true, force: true });

try {
  mkdirSync(join(stagingDir, "resources"), { recursive: true });
  copyFileSync(appExe, join(stagingDir, basename(appExe)));
  copyFileSync(bundledTdl, join(stagingDir, "resources", "tdl.exe"));
  createPortableZip(stagingDir, portableZip);
} finally {
  rmSync(stagingDir, { recursive: true, force: true });
}

copyFileSync(msi, installerMsi);

console.log(`Release artifacts copied to ${releaseDir}`);
for (const artifact of [portableZip, installerMsi]) {
  console.log(`- ${basename(artifact)} (${statSync(artifact).size} bytes)`);
}

function assertExists(path, message) {
  if (!existsSync(path)) {
    throw new Error(message);
  }
}

function cleanReleaseDir() {
  for (const file of readdirSync(releaseDir)) {
    const path = join(releaseDir, file);
    if (file === basename(stagingDir)) {
      rmSync(path, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
      continue;
    }
    if (/\.(exe|msi|zip)$/i.test(file)) {
      rmSync(path, { force: true, maxRetries: 5, retryDelay: 200 });
    }
  }
}

function createPortableZip(sourceDir, destination) {
  execFileSync(
    "powershell.exe",
    [
      "-NoProfile",
      "-Command",
      `$items = Get-ChildItem -LiteralPath ${psQuote(sourceDir)}; Compress-Archive -LiteralPath $items.FullName -DestinationPath ${psQuote(destination)} -Force`,
    ],
    { stdio: "inherit" },
  );
}

function psQuote(value) {
  return `'${value.replaceAll("'", "''")}'`;
}

function releasePlatform() {
  if (process.platform !== "win32") {
    throw new Error(`Windows release artifacts must be collected on Windows, got ${process.platform}`);
  }

  if (process.arch === "x64") return "windows_x86_64";
  if (process.arch === "arm64") return "windows_arm64";
  if (process.arch === "ia32") return "windows_x86";
  if (process.arch === "arm") return "windows_armv7";

  throw new Error(`Unsupported Windows architecture: ${process.arch}`);
}

function slugify(value) {
  const slug = String(value)
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  if (!slug) {
    throw new Error("Cannot derive release artifact slug from package name");
  }
  return slug;
}

function filesIn(directory, extension) {
  if (!existsSync(directory)) {
    return [];
  }

  return readdirSync(directory)
    .filter((file) => file.toLowerCase().endsWith(extension))
    .map((file) => join(directory, file));
}

function newestFile(files) {
  return [...files].sort((left, right) => statSync(right).mtimeMs - statSync(left).mtimeMs)[0];
}
