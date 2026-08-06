export interface HandoffFile {
  path: string;
  added: number;
  removed: number;
}

export interface HandoffContent {
  text: string;
  files: HandoffFile[];
  includeText: boolean;
  includeFiles: boolean;
  note: string;
}

export function buildHandoffBody(content: HandoffContent): string {
  const parts: string[] = [];
  if (content.note.trim()) parts.push(content.note.trim());
  if (content.includeText && content.text.trim()) parts.push(content.text.trim());
  if (content.includeFiles && content.files.length) {
    parts.push([
      "Files changed:",
      ...content.files.map(
        (file) => `- ${file.path} (+${file.added} -${file.removed})`,
      ),
    ].join("\n"));
  }
  return parts.join("\n\n");
}

export function buildHandoffPrompt(
  sourceAgent: string,
  sourceSession: string,
  body: string,
): string {
  return [
    `## Context handoff from ${sourceAgent.trim()}`,
    `Source session: ${sourceSession.trim()}`,
    "",
    body.trim(),
  ].join("\n");
}

export function parseHandoffPrompt(
  value?: string,
): { source: string; body: string } | null {
  const text = value?.replace(/\r\n?/g, "\n") ?? "";
  const match = text.match(
    /^## Context handoff from (.+)\n(?:Source session: .+\n)?\n([\s\S]+)$/,
  );
  return match ? { source: match[1].trim(), body: match[2].trim() } : null;
}
