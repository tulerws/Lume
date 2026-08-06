import type { PromptAttachment } from "$lib/domain";

const lumeAttachedFilesMarker = "Files attached through Lume. Inspect these local paths:";
const imageExtensions = new Set(["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "avif"]);
const explicitDeliveryCue = /\b(?:download|downloadable|baixar|baixe|arquivo\s+(?:final|gerado|para\s+baixar)|ficheiro\s+(?:final|gerado)|final\s+(?:files?|pdf|image|document|build)|generated\s+(?:files?|pdf|image|document|build)|pdf\s+final|output\s*(?:file)?|deliverable|attachment|anexo|export(?:ed|ado)?|saved\s+(?:at|to)|salv[oa]\s+em|available\s+(?:here|for\s+download)|dispon[ií]vel\s+(?:aqui|para\s+baixar)|resultado\s+final|entrega\s+final)\b/i;

export interface ResponseFileReference {
  path: string;
  name: string;
  mimeType: string;
  isImage: boolean;
}

export function cleanPromptTransport(value?: string): string {
  const normalized = String(value ?? "").replace(/\r\n?/g, "\n");
  const marker = normalized.indexOf(lumeAttachedFilesMarker);
  return (marker >= 0 ? normalized.slice(0, marker) : normalized).trim();
}

export function promptTextKey(value?: string): string {
  return cleanPromptTransport(value)
    .replace(/[ \t]+$/gm, "")
    .trim();
}

export function extractResponseFiles(
  text: string | undefined,
  attachments: PromptAttachment[] = [],
  workingDirectory?: string,
): ResponseFileReference[] {
  const candidates: Array<{ path: string; name?: string; mimeType?: string }> = [];
  const source = String(text ?? "");
  for (const match of source.matchAll(/!?\[[^\]\n]*\]\((<[^>\n]+>|[^)\n]+)\)/g)) {
    const path = markdownTargetPath(match[1]);
    if (!path || !isExplicitResponseFile(source, match)) continue;
    const attachment = attachments.find((candidate) =>
      candidate.path && sameResponsePath(candidate.path, path, workingDirectory)
    );
    candidates.push({
      path,
      name: attachment?.name,
      mimeType: attachment?.mimeType,
    });
  }
  for (const line of source.split(/\r?\n/)) {
    if (!explicitDeliveryCue.test(line)) continue;
    for (const match of line.matchAll(/`([^`\n]+)`/g)) {
      if (isPathLike(match[1])) candidates.push({ path: match[1] });
    }
    if (/\[[^\]\n]*\]\(/.test(line)) continue;
    const separator = line.indexOf(":");
    const tail = (separator >= 0 ? line.slice(separator + 1) : line)
      .trim()
      .replace(/^['"<`]+|['">`]+$/g, "");
    if (isPathLike(tail)) candidates.push({ path: tail });
  }

  const result: ResponseFileReference[] = [];
  const seen = new Set<string>();
  for (const candidate of candidates) {
    const path = resolveResponsePath(candidate.path, workingDirectory);
    if (!path || isSensitivePath(path)) continue;
    const key = path.replace(/\\/g, "/").toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    const name = candidate.name || fileName(path);
    const mimeType = candidate.mimeType || mimeForFile(name);
    result.push({
      path,
      name,
      mimeType,
      isImage: mimeType.startsWith("image/") || imageExtensions.has(extension(name)),
    });
    if (result.length === 8) break;
  }
  return result;
}

function isExplicitResponseFile(
  source: string,
  match: RegExpMatchArray,
): boolean {
  if (match[0].startsWith("!")) return true;
  const index = match.index ?? 0;
  const lineStart = source.lastIndexOf("\n", index - 1) + 1;
  const lineEnd = source.indexOf("\n", index + match[0].length);
  const line = source.slice(lineStart, lineEnd < 0 ? source.length : lineEnd);
  const labelEnd = match[0].indexOf("]");
  const label = labelEnd > 1 ? match[0].slice(match[0].startsWith("!") ? 2 : 1, labelEnd) : "";
  return explicitDeliveryCue.test(`${label} ${line.slice(0, Math.max(0, index - lineStart))}`);
}

function sameResponsePath(left: string, right: string, workingDirectory?: string): boolean {
  const leftPath = resolveResponsePath(left, workingDirectory);
  const rightPath = resolveResponsePath(right, workingDirectory);
  return Boolean(leftPath && rightPath && leftPath.replace(/\\/g, "/").toLowerCase()
    === rightPath.replace(/\\/g, "/").toLowerCase());
}

function markdownTargetPath(raw: string): string {
  const value = raw.trim();
  if (value.startsWith("<")) {
    const end = value.indexOf(">");
    return end > 0 ? value.slice(1, end) : "";
  }
  return value
    .replace(/\s+["'][^"']*["']\s*$/, "")
    .replace(/%20/g, " ");
}

function resolveResponsePath(raw: string, workingDirectory?: string): string {
  let value = String(raw ?? "")
    .trim()
    .replace(/^file:\/\//i, "")
    .replace(/:(\d+)$/, "")
    .replace(/^[`"'<>]+|[`"'<>.,;:]+$/g, "");
  if (!value || /^(?:https?:|mailto:|data:|#)/i.test(value)) return "";
  if (isAbsolutePath(value)) return value;
  if (!workingDirectory || !isPathLike(value)) return "";
  const separator = workingDirectory.includes("\\") ? "\\" : "/";
  value = value.replace(/[\\/]/g, separator);
  if (value.startsWith(`.${separator}`)) value = value.slice(2);
  return `${workingDirectory.replace(/[\\/]+$/, "")}${separator}${value}`;
}

function isAbsolutePath(value: string): boolean {
  return value.startsWith("/") || value.startsWith("\\\\") || /^[A-Za-z]:[\\/]/.test(value);
}

function isPathLike(value: string): boolean {
  const clean = value.trim();
  return isAbsolutePath(clean) || /^\.{0,2}[\\/]/.test(clean) || /(?:^|[\\/])[^\\/]+\.[A-Za-z0-9]{1,10}$/.test(clean);
}

function fileName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() || path;
}

function extension(path: string): string {
  const name = fileName(path);
  const index = name.lastIndexOf(".");
  return index > 0 ? name.slice(index + 1).toLowerCase() : "";
}

function mimeForFile(path: string): string {
  return ({
    png: "image/png",
    jpg: "image/jpeg",
    jpeg: "image/jpeg",
    gif: "image/gif",
    webp: "image/webp",
    svg: "image/svg+xml",
    bmp: "image/bmp",
    avif: "image/avif",
    pdf: "application/pdf",
    json: "application/json",
    txt: "text/plain",
    md: "text/markdown",
    csv: "text/csv",
    zip: "application/zip",
  } as Record<string, string>)[extension(path)] || "application/octet-stream";
}

function isSensitivePath(path: string): boolean {
  const normalized = path.replace(/\\/g, "/").toLowerCase();
  const name = fileName(normalized);
  return /\/(?:\.ssh|\.gnupg|\.aws)(?:\/|$)/.test(normalized)
    || name === ".env"
    || name.startsWith(".env.")
    || ["id_rsa", "id_ed25519", ".netrc", ".npmrc", ".pypirc"].includes(name)
    || [".pem", ".key", ".p12", ".pfx"].some((suffix) => name.endsWith(suffix));
}
