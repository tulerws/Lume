import assert from "node:assert/strict";
import {
  cleanPromptTransport,
  extractResponseFiles,
  promptTextKey,
} from "../src/lib/chatAttachments.ts";

const transportedPrompt = `Create the Rio presentation

Files attached through Lume. Inspect these local paths:
- "/home/user/Downloads/Mural de Troca — Apresentação Comercial.pdf"`;

assert.equal(cleanPromptTransport(transportedPrompt), "Create the Rio presentation");
assert.equal(promptTextKey(transportedPrompt), promptTextKey("Create the Rio presentation"));

const files = extractResponseFiles(
  "Final files: [deck](outputs/Rio presentation.pdf), [preview](/work/mural/preview.png)\nThe editable generator is in `generate.mjs`, and the [site](https://example.com/file.pdf) is online.",
  [{
    id: "preview",
    name: "preview.png",
    mimeType: "image/png",
    previewDataUrl: "",
    path: "/work/mural/preview.png",
  }],
  "/work/mural",
);

assert.deepEqual(files, [
  {
    path: "/work/mural/outputs/Rio presentation.pdf",
    name: "Rio presentation.pdf",
    mimeType: "application/pdf",
    isImage: false,
  },
  {
    path: "/work/mural/preview.png",
    name: "preview.png",
    mimeType: "image/png",
    isImage: true,
  },
]);

assert.deepEqual(
  extractResponseFiles("[secret](/work/mural/.env)", [], "/work/mural"),
  [],
);

assert.deepEqual(
  extractResponseFiles(
    "The editable generator is in `generate.mjs`.",
    [{
      id: "generator",
      name: "generate.mjs",
      mimeType: "application/octet-stream",
      previewDataUrl: "",
      path: "/work/mural/generate.mjs",
    }],
    "/work/mural",
  ),
  [],
);

assert.deepEqual(
  extractResponseFiles(
    "Main files: [workflow_runtime.rs](/work/lume/src/workflow_runtime.rs) and [WorkflowBridgeWindow.svelte](/work/lume/src/WorkflowBridgeWindow.svelte).",
    [],
    "/work/lume",
  ),
  [],
);

assert.deepEqual(
  extractResponseFiles(
    "Generated file: [workflow bundle](/work/lume/workflow_bundle.rs)",
    [],
    "/work/lume",
  ),
  [{
    path: "/work/lume/workflow_bundle.rs",
    name: "workflow_bundle.rs",
    mimeType: "application/octet-stream",
    isImage: false,
  }],
);

assert.deepEqual(
  extractResponseFiles(
    "PDF final: `/work/mural/outputs/Rio presentation.pdf`",
    [],
    "/work/mural",
  ),
  [{
    path: "/work/mural/outputs/Rio presentation.pdf",
    name: "Rio presentation.pdf",
    mimeType: "application/pdf",
    isImage: false,
  }],
);

console.log("chat attachment test suite passed");
