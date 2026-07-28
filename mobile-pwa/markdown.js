/** @type {Record<string, string>} */
const htmlEscapes = {
  "&": "&amp;",
  "<": "&lt;",
  ">": "&gt;",
  '"': "&quot;",
  "'": "&#039;",
};

/** @param {unknown} value */
function escapeHtml(value) {
  return String(value ?? "").replace(/[&<>"']/g, (character) => htmlEscapes[character]);
}

/** @param {unknown} value */
function safeLink(value) {
  const href = String(value ?? "").trim();
  return /^(?:https?:|mailto:)/i.test(href) ? href : null;
}

/** @param {string} value */
function formatEmphasis(value) {
  return value
    .replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>")
    .replace(/__([^_\n]+)__/g, "<strong>$1</strong>")
    .replace(/~~([^~\n]+)~~/g, "<del>$1</del>")
    .replace(/(^|[\s(])\*([^*\n]+)\*(?=$|[\s).,!?:;])/g, "$1<em>$2</em>")
    .replace(/(^|[\s(])_([^_\n]+)_(?=$|[\s).,!?:;])/g, "$1<em>$2</em>");
}

/** @param {string} source */
function renderInline(source) {
  /** @type {string[]} */
  const tokens = [];
  /** @param {string} html */
  const stash = (html) => {
    const index = tokens.push(html) - 1;
    return `\uE000${index}\uE001`;
  };

  let value = source
    .replace(/`([^`\n]+)`/g, (_, code) => stash(`<code>${escapeHtml(code)}</code>`))
    .replace(/<(https?:\/\/[^>\s]+)>/gi, (_, href) => {
      const safeHref = safeLink(href);
      if (!safeHref) return escapeHtml(href);
      return stash(
        `<a href="${escapeHtml(safeHref)}" target="_blank" rel="noopener noreferrer">${escapeHtml(href)}</a>`,
      );
    })
    .replace(
      /\[([^\]\n]+)\]\(([^)\s]+)(?:\s+["'][^"']*["'])?\)/g,
      (match, label, href) => {
        const safeHref = safeLink(href);
        if (!safeHref) return label;
        return stash(
          `<a href="${escapeHtml(safeHref)}" target="_blank" rel="noopener noreferrer">${formatEmphasis(escapeHtml(label))}</a>`,
        );
      },
    );

  value = formatEmphasis(escapeHtml(value));
  return value.replace(/\uE000(\d+)\uE001/g, (_, index) => tokens[Number(index)] ?? "");
}

/** @param {string} line */
function startsBlock(line) {
  return /^\s*(?:```|#{1,4}\s+|>\s?|[-*+]\s+|\d+\.\s+|(?:---+|\*\*\*+)\s*$)/.test(line);
}

/**
 * Render the supported Markdown subset after escaping all source HTML.
 * Only generated markup and explicitly allowed http(s)/mailto links survive.
 * @param {unknown} markdown
 */
export function renderSafeMarkdown(markdown) {
  const source = String(markdown ?? "").replace(/\r\n?/g, "\n").trim();
  if (!source) return "";

  const lines = source.split("\n");
  /** @type {string[]} */
  const output = [];
  let index = 0;

  while (index < lines.length) {
    const line = lines[index];
    if (!line.trim()) {
      index += 1;
      continue;
    }

    const fence = line.match(/^\s*```([a-z0-9_+-]*)\s*$/i);
    if (fence) {
      const language = fence[1] ? ` class="language-${fence[1].toLowerCase()}"` : "";
      const code = [];
      index += 1;
      while (index < lines.length && !/^\s*```\s*$/.test(lines[index])) {
        code.push(lines[index]);
        index += 1;
      }
      if (index < lines.length) index += 1;
      output.push(`<pre><code${language}>${escapeHtml(code.join("\n"))}</code></pre>`);
      continue;
    }

    const heading = line.match(/^\s*(#{1,4})\s+(.+)$/);
    if (heading) {
      const level = heading[1].length;
      output.push(`<h${level}>${renderInline(heading[2].trim())}</h${level}>`);
      index += 1;
      continue;
    }

    if (/^\s*(?:---+|\*\*\*+)\s*$/.test(line)) {
      output.push("<hr>");
      index += 1;
      continue;
    }

    if (/^\s*>\s?/.test(line)) {
      const quote = [];
      while (index < lines.length && /^\s*>\s?/.test(lines[index])) {
        quote.push(lines[index].replace(/^\s*>\s?/, ""));
        index += 1;
      }
      output.push(`<blockquote>${quote.map(renderInline).join("<br>")}</blockquote>`);
      continue;
    }

    const unordered = line.match(/^\s*[-*+]\s+(.+)$/);
    if (unordered) {
      const items = [];
      while (index < lines.length) {
        const item = lines[index].match(/^\s*[-*+]\s+(.+)$/);
        if (!item) break;
        items.push(`<li>${renderInline(item[1])}</li>`);
        index += 1;
      }
      output.push(`<ul>${items.join("")}</ul>`);
      continue;
    }

    const ordered = line.match(/^\s*\d+\.\s+(.+)$/);
    if (ordered) {
      const items = [];
      while (index < lines.length) {
        const item = lines[index].match(/^\s*\d+\.\s+(.+)$/);
        if (!item) break;
        items.push(`<li>${renderInline(item[1])}</li>`);
        index += 1;
      }
      output.push(`<ol>${items.join("")}</ol>`);
      continue;
    }

    const paragraph = [line.trim()];
    index += 1;
    while (index < lines.length && lines[index].trim() && !startsBlock(lines[index])) {
      paragraph.push(lines[index].trim());
      index += 1;
    }
    output.push(`<p>${paragraph.map(renderInline).join("<br>")}</p>`);
  }

  return output.join("");
}
