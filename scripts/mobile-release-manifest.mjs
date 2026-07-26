import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { basename } from "node:path";

const values = new Map();
for (let index = 2; index < process.argv.length; index += 2) {
  values.set(process.argv[index], process.argv[index + 1]);
}

const required = ["--version", "--apk", "--repository", "--tag", "--output"];
for (const option of required) {
  if (!values.get(option)) throw new Error(`Missing ${option}`);
}

const version = values.get("--version");
const apkPath = values.get("--apk");
const repository = values.get("--repository");
const tag = values.get("--tag");
const output = values.get("--output");
const apkName = basename(apkPath);
const apk = await readFile(apkPath);
const sha256 = createHash("sha256").update(apk).digest("hex");

const manifest = {
  version,
  publishedAt: new Date().toISOString(),
  notesUrl: `https://github.com/${repository}/releases/tag/${tag}`,
  android: {
    url: `https://github.com/${repository}/releases/download/${tag}/${apkName}`,
    sha256,
  },
};

await writeFile(output, `${JSON.stringify(manifest, null, 2)}\n`);
