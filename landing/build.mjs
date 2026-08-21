import { copyFile, mkdir, rm } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const landingDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryDirectory = path.dirname(landingDirectory);
const outputDirectory = path.join(repositoryDirectory, "landing-dist");
const assetsDirectory = path.join(outputDirectory, "assets");

const sourceFiles = ["index.html", "styles.css", "app.js"];
const assets = [
  ["static/branding/dark/lume-128.png", "lume.png"],
  ["landing/assets/lume-mascot-sprites-v3.png", "lume-mascot-sprites-v3.png"],
  ["landing/assets/lume-mascot-reading.png", "lume-mascot-reading.png"],
];

await rm(outputDirectory, { recursive: true, force: true });
await mkdir(assetsDirectory, { recursive: true });

await Promise.all([
  ...sourceFiles.map((file) =>
    copyFile(path.join(landingDirectory, file), path.join(outputDirectory, file)),
  ),
  ...assets.map(([source, destination]) =>
    copyFile(path.join(repositoryDirectory, source), path.join(assetsDirectory, destination)),
  ),
]);

console.log(`Lume landing page built at ${outputDirectory}`);
