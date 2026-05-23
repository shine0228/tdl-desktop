import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const helperDir = resolve(root, "helper", "tdl-helper");
const output = resolve(root, "src-tauri", "resources", "tdl-helper.exe");

mkdirSync(dirname(output), { recursive: true });

const env = {
  ...process.env,
  GOSUMDB: process.env.GOSUMDB || "off",
};

execFileSync(
  "go",
  ["build", "-trimpath", "-ldflags=-s -w", "-o", output, "."],
  {
    cwd: helperDir,
    env,
    stdio: "inherit",
  },
);

console.log(`tdl-helper built: ${output}`);
