import { access, readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const mobileRoot = resolve(root, "mobile-pwa");
const errors = [];

async function requireFile(path, context) {
  try {
    await access(path);
  } catch {
    errors.push(`${context}: missing ${path.slice(root.length + 1)}`);
  }
}

async function requireMatchingFiles(source, mobile) {
  const sourcePath = resolve(root, source);
  const mobilePath = resolve(root, mobile);
  await requireFile(sourcePath, source);
  await requireFile(mobilePath, source);
  try {
    const [sourceContent, mobileContent] = await Promise.all([
      readFile(sourcePath, "utf8"),
      readFile(mobilePath, "utf8"),
    ]);
    if (sourceContent !== mobileContent) {
      errors.push(`${mobile} is out of sync with ${source}`);
    }
  } catch {
    // Missing files are reported above.
  }
}

await requireMatchingFiles("src/lib/markdown.js", "mobile-pwa/markdown.js");
await requireMatchingFiles("src/lib/responseDedup.js", "mobile-pwa/responseDedup.js");

for (const entry of ["app.js", "markdown.js", "responseDedup.js", "sw.js"]) {
  const path = resolve(mobileRoot, entry);
  await requireFile(path, entry);
  let source = "";
  try {
    source = await readFile(path, "utf8");
  } catch {
    continue;
  }
  const imports = [
    ...source.matchAll(/\bfrom\s+["'](\.\/[^"']+)["']/g),
    ...source.matchAll(/\bimport\s+["'](\.\/[^"']+)["']/g),
  ];
  for (const match of imports) {
    await requireFile(resolve(dirname(path), match[1]), `${entry} import`);
  }
}

const serviceWorker = await readFile(resolve(mobileRoot, "sw.js"), "utf8");
for (const match of serviceWorker.matchAll(/["'](\.\/[^"']+)["']/g)) {
  if (match[1] === "./") continue;
  await requireFile(resolve(mobileRoot, match[1]), "service worker cache");
}

if (errors.length) {
  console.error(errors.map((error) => `- ${error}`).join("\n"));
  process.exit(1);
}

console.log("Mobile assets are complete and synchronized.");
