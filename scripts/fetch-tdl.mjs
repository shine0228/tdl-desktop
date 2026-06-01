import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, rmSync, statSync, writeFileSync, copyFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, basename, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outFile = resolve(repoRoot, "src-tauri/resources/tdl.exe");

const DEFAULT_SOURCES = [
  "https://api.github.com/repos/iyear/tdl/releases/latest",
];

function resolveSources() {
  const sources = [...DEFAULT_SOURCES];
  if (process.env.TDL_MIRROR) {
    sources.unshift(process.env.TDL_MIRROR);
  }
  return sources;
}

function assetName() {
  const arch = process.arch;
  if (arch === "x64") return "tdl_Windows_64bit.zip";
  if (arch === "ia32") return "tdl_Windows_32bit.zip";
  if (arch === "arm64") return "tdl_Windows_arm64.zip";
  if (arch === "arm") return "tdl_Windows_armv7.zip";
  return "tdl_Windows_64bit.zip";
}

function psQuote(value) {
  return `'${value.replaceAll("'", "''")}'`;
}

function findFile(dir, fileName) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const fullPath = join(dir, entry.name);
    if (entry.isDirectory()) {
      const found = findFile(fullPath, fileName);
      if (found) return found;
    } else if (entry.isFile() && entry.name.toLowerCase() === fileName.toLowerCase()) {
      return fullPath;
    }
  }
  return null;
}

function expectedSha256(asset) {
  const digest = asset.digest;
  if (typeof digest !== "string" || !digest.startsWith("sha256:")) {
    throw new Error(`Release asset ${asset.name} does not provide a SHA-256 digest`);
  }
  const value = digest.slice("sha256:".length).trim().toLowerCase();
  if (!/^[a-f0-9]{64}$/.test(value)) {
    throw new Error(`Release asset ${asset.name} has an invalid SHA-256 digest`);
  }
  return value;
}

function verifySha256(buffer, expected, label) {
  const actual = createHash("sha256").update(buffer).digest("hex");
  if (actual !== expected) {
    throw new Error(`SHA-256 mismatch for ${label}: expected ${expected}, got ${actual}`);
  }
  console.log(`Verified SHA-256 for ${label}: ${actual}`);
}

if (existsSync(outFile) && statSync(outFile).size > 0 && !process.env.FORCE_TDL_DOWNLOAD) {
  console.log(`tdl.exe already exists: ${outFile}`);
  process.exit(0);
}

const sources = resolveSources();
let release;
let lastError;

for (const apiUrl of sources) {
  try {
    console.log(`Trying release source: ${apiUrl}`);
    const releaseResp = await fetch(apiUrl, {
      headers: {
        "User-Agent": "TDL-Desktop",
        Accept: "application/vnd.github+json",
      },
    });

    if (!releaseResp.ok) {
      console.warn(`Source ${apiUrl} returned ${releaseResp.status} ${releaseResp.statusText}, trying next...`);
      lastError = new Error(`Failed to fetch tdl release metadata from ${apiUrl}: ${releaseResp.status} ${releaseResp.statusText}`);
      continue;
    }

    release = await releaseResp.json();
    console.log(`Successfully fetched release from ${apiUrl}`);
    break;
  } catch (error) {
    console.warn(`Source ${apiUrl} failed: ${error.message}, trying next...`);
    lastError = error;
  }
}

if (!release) {
  throw new Error(`All tdl release sources failed. Last error: ${lastError?.message ?? "unknown"}`);
}
const wanted = assetName();
const asset = release.assets?.find((item) => item.name === wanted);

if (!asset) {
  const available = release.assets?.map((item) => item.name).join(", ") ?? "none";
  throw new Error(`Asset not found: ${wanted}. Available: ${available}`);
}

const expectedDigest = expectedSha256(asset);
console.log(`Downloading ${wanted}...`);
const archiveResp = await fetch(asset.browser_download_url, {
  headers: { "User-Agent": "TDL-Desktop" },
});

if (!archiveResp.ok) {
  throw new Error(`Failed to download ${wanted}: ${archiveResp.status} ${archiveResp.statusText}`);
}

const archiveBuffer = Buffer.from(await archiveResp.arrayBuffer());
verifySha256(archiveBuffer, expectedDigest, wanted);

const tempDir = join(tmpdir(), `tdl-desktop-${Date.now()}`);
const archivePath = join(tempDir, basename(wanted));
const extractDir = join(tempDir, "extract");
mkdirSync(extractDir, { recursive: true });
writeFileSync(archivePath, archiveBuffer);

execFileSync(
  "powershell.exe",
  [
    "-NoProfile",
    "-ExecutionPolicy",
    "Bypass",
    "-Command",
    `Expand-Archive -LiteralPath ${psQuote(archivePath)} -DestinationPath ${psQuote(extractDir)} -Force`,
  ],
  { stdio: "inherit" },
);

const extracted = findFile(extractDir, "tdl.exe");
if (!extracted) {
  throw new Error("tdl.exe not found in downloaded archive");
}

mkdirSync(dirname(outFile), { recursive: true });
copyFileSync(extracted, outFile);
rmSync(tempDir, { recursive: true, force: true });

console.log(`Bundled tdl.exe written to ${outFile}`);
