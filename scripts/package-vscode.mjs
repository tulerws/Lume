import { spawnSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const extension = resolve(root, "extensions/vscode");
const output = resolve(root, "src-tauri/resources/lume-vscode.vsix");
mkdirSync(dirname(output), { recursive: true });

const executable = resolve(root, "node_modules/.bin/vsce");
const result = spawnSync(
  executable,
  ["package", "--no-dependencies", "--skip-license", "--out", output],
  { cwd: extension, stdio: "inherit" },
);
process.exit(result.status ?? 1);
