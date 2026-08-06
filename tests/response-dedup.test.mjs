import assert from "node:assert/strict";
import {
  latestResponseText,
  sameResponseText,
} from "../src/lib/responseDedup.js";

const compact = `The correct fix is to keep the Markdown renderer and package it with the mobile app.
The supported formatting includes:
- Code blocks- Lists and headings- Bold text and links- Line breaks`;
const formatted = `The correct fix is to keep the Markdown renderer and package it with the mobile app.
The supported formatting includes:

- Code blocks
- Lists and headings
- Bold text and links
- Line breaks`;

assert.equal(
  sameResponseText(compact, formatted),
  true,
  "formatting-only revisions should be deduplicated",
);
assert.equal(
  sameResponseText(
    formatted,
    `${formatted}\n\nThe mobile build now validates these files before publishing.`,
  ),
  true,
  "a refined final version of the same response should replace the earlier one",
);
assert.equal(
  sameResponseText(
    "I am inspecting the clipboard integration before changing the terminal.",
    "The clipboard fix is complete and the Android package now includes Markdown.",
  ),
  false,
  "different agent messages must remain visible",
);
assert.equal(
  sameResponseText("a short response", "a short response with a different conclusion"),
  false,
  "short partial matches must not be collapsed",
);
assert.equal(
  latestResponseText("older response", "final response", 10, 20),
  "final response",
  "the newest matching response should win",
);

console.log("response deduplication test suite passed");
