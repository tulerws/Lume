export interface FileChangeSummary {
  path: string;
  added: number;
  removed: number;
}

export function displayFileChangePath(path: string): string {
  const normalized = path.trim();
  const isExternal =
    /^(?:[a-z]:[\\/]|[\\/]{1,2}|\.\.[\\/])/i.test(normalized);
  if (isExternal) return normalized;
  return normalized.split(/[\\/]/).pop() || normalized;
}

function cleanPath(value: string, workingDirectory?: string): string | null {
  let path = value.trim().replace(/^["']|["']$/g, "");
  const patchPath = path.match(
    /\*\*\*\s+(?:Update|Add|Delete)\s+File:\s+(.+?)(?=\s+(?:\*\*\*|@@)|$)/,
  );
  if (patchPath) path = patchPath[1].trim();
  path = path.split(/\s@@\s/, 1)[0].split(/\s\*\*\*\s/, 1)[0].trim();
  if (/[\r\n]/.test(path)) return null;
  if (path === "/dev/null") return null;
  if (path.startsWith("a/") || path.startsWith("b/")) path = path.slice(2);
  const root = workingDirectory?.replace(/[\\/]+$/, "");
  if (root && (path === root || path.startsWith(`${root}/`) || path.startsWith(`${root}\\`))) {
    path = path.slice(root.length).replace(/^[\\/]+/, "");
  }
  return path || null;
}

function record(
  summaries: Map<string, FileChangeSummary>,
  path: string | null,
  added = 0,
  removed = 0,
) {
  if (!path) return;
  const current = summaries.get(path);
  if (current) {
    current.added = Math.max(current.added, added);
    current.removed = Math.max(current.removed, removed);
  } else {
    summaries.set(path, { path, added, removed });
  }
}

export function summarizeFileChanges(
  detail: string,
  reportedFiles: string[],
  workingDirectory?: string,
): FileChangeSummary[] {
  const summaries = new Map<string, FileChangeSummary>();
  let currentPath: string | null = null;
  let added = 0;
  let removed = 0;
  let counting = false;

  const flush = () => {
    record(summaries, currentPath, added, removed);
    added = 0;
    removed = 0;
  };

  for (const line of detail.split(/\r?\n/)) {
    const patchHeader = line.match(/^\*\*\* (?:Update|Add|Delete) File:\s+(.+)$/);
    const gitHeader = line.match(/^diff --git a\/(.+?) b\/(.+)$/);
    const nextFile = line.match(/^\+\+\+\s+(?:b\/)?(.+)$/);
    if (patchHeader || gitHeader || nextFile) {
      flush();
      currentPath = cleanPath(
        patchHeader?.[1] ?? gitHeader?.[2] ?? nextFile?.[1] ?? "",
        workingDirectory,
      );
      counting = Boolean(patchHeader);
      continue;
    }
    if (line.startsWith("@@")) {
      counting = true;
      continue;
    }
    if (line === "*** End Patch") {
      flush();
      currentPath = null;
      counting = false;
      continue;
    }
    if (!currentPath || !counting) continue;
    if (line.startsWith("+") && !line.startsWith("+++")) added += 1;
    if (line.startsWith("-") && !line.startsWith("---")) removed += 1;
  }
  flush();

  const inlinePatch =
    /\*\*\*\s+(?:Update|Add|Delete)\s+File:\s+(.+?)(?=\s+(?:\*\*\*|@@)|[\r\n]|$)/g;
  for (const match of detail.matchAll(inlinePatch)) {
    record(summaries, cleanPath(match[1], workingDirectory));
  }

  for (const reported of reportedFiles) {
    if (
      reported.includes("\n") ||
      reported.includes("*** Begin Patch") ||
      /\*\*\*\s+(?:Update|Add|Delete)\s+File:/.test(reported)
    ) {
      for (const summary of summarizeFileChanges(reported, [], workingDirectory)) {
        record(summaries, summary.path, summary.added, summary.removed);
      }
      continue;
    }
    record(summaries, cleanPath(reported, workingDirectory));
  }
  return [...summaries.values()].filter(
    (change) => change.added > 0 || change.removed > 0,
  );
}

export function mergeFileChanges(
  target: FileChangeSummary[],
  incoming: FileChangeSummary[],
): void {
  for (const change of incoming) {
    const current = target.find((item) => item.path === change.path);
    if (current) {
      current.added = Math.max(current.added, change.added);
      current.removed = Math.max(current.removed, change.removed);
    } else {
      target.push({ ...change });
    }
  }
}
