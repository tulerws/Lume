import assert from "node:assert/strict";
import { renderSafeMarkdown, stripInternalAgentMetadata } from "../src/lib/markdown.js";
import { renderSafeMarkdown as renderMobileMarkdown } from "../mobile-pwa/markdown.js";

const rendered = renderSafeMarkdown(`Resposta visível.

<oai-mem-citation>
<citation_entries>
MEMORY.md:1-2|note=[internal]
</citation_entries>
<rollout_ids>
019f8061-7032-7521-b333-84f84c744fa8
</rollout_ids>
</oai-mem-citation>`);

assert.equal(rendered, "<p>Resposta visível.</p>");
assert.equal(rendered.includes("oai-mem-citation"), false);
assert.equal(stripInternalAgentMetadata("Visible\n<oai-mem-citation>hidden"), "Visible");
assert.equal(
  renderSafeMarkdown("Arquivo: [README](https://example.com/README.md)"),
  '<p>Arquivo: <a href="https://example.com/README.md" target="_blank" rel="noopener noreferrer">README</a></p>',
);

const table = renderSafeMarkdown(`| Phase | Status |
| :--- | ---: |
| Context Builder | Complete |
| Safety | **Testing** |`);
assert.match(table, /<div class="markdown-table-wrap"><table>/);
assert.match(table, /<th class="align-left">Phase<\/th>/);
assert.match(table, /<th class="align-right">Status<\/th>/);
assert.match(table, /<td class="align-right"><strong>Testing<\/strong><\/td>/);
assert.equal(renderMobileMarkdown(`| Phase | Status |\n| :--- | ---: |\n| Context Builder | Complete |\n| Safety | **Testing** |`), table);
assert.equal(renderSafeMarkdown("A | B\nStill text"), "<p>A | B<br>Still text</p>");

console.log("markdown test suite passed");
