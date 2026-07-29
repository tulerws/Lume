/** @param {string | null | undefined} value */
function chatTextKey(value) {
  return String(value ?? "")
    .replace(/\r\n?/g, "\n")
    .replace(/[ \t]+$/gm, "")
    .trim();
}

const semanticSeparator = (() => {
  try {
    return new RegExp("[^\\p{L}\\p{N}]+", "gu");
  } catch {
    return /[^a-z0-9\u00c0-\u024f\u0370-\u052f\u1e00-\u1eff\u3040-\u30ff\u3400-\u9fff]+/gi;
  }
})();

/** @param {string | null | undefined} value */
function comparableResponseText(value) {
  return chatTextKey(value).replace(/(?:…|\.\.\.)$/, "").trimEnd();
}

/** @param {string | null | undefined} value */
function semanticResponseText(value) {
  return comparableResponseText(value)
    .normalize("NFKC")
    .toLocaleLowerCase("en-US")
    .replace(/```[a-z0-9_+-]*/gi, " ")
    .replace(semanticSeparator, " ")
    .trim()
    .replace(/\s+/g, " ");
}

/** @param {string | null | undefined} value */
function semanticCompactText(value) {
  return semanticResponseText(value).replace(/\s+/g, "");
}

/** @param {string | null | undefined} value */
function responseWordSet(value) {
  return new Set(
    semanticResponseText(value)
      .split(" ")
      .filter((word) => word.length >= 3),
  );
}

/**
 * @param {string | null | undefined} left
 * @param {string | null | undefined} right
 */
export function sameResponseText(left, right) {
  const leftKey = comparableResponseText(left);
  const rightKey = comparableResponseText(right);
  if (!leftKey || !rightKey) return false;
  if (leftKey === rightKey) return true;

  const [shorter, longer] =
    leftKey.length <= rightKey.length ? [leftKey, rightKey] : [rightKey, leftKey];
  if (shorter.length >= 256 && longer.startsWith(shorter)) return true;

  const leftCompact = semanticCompactText(left);
  const rightCompact = semanticCompactText(right);
  if (!leftCompact || !rightCompact) return false;
  if (leftCompact === rightCompact) return true;

  const [compactShorter, compactLonger] = leftCompact.length <= rightCompact.length
    ? [leftCompact, rightCompact]
    : [rightCompact, leftCompact];
  if (compactShorter.length >= 120 && compactLonger.startsWith(compactShorter)) {
    return true;
  }

  const leftWords = responseWordSet(left);
  const rightWords = responseWordSet(right);
  if (Math.min(leftWords.size, rightWords.size) < 12) return false;
  const sharedWords = [...leftWords].filter((word) => rightWords.has(word)).length;
  const coverage = sharedWords / Math.min(leftWords.size, rightWords.size);
  const lengthRatio =
    Math.min(leftCompact.length, rightCompact.length)
    / Math.max(leftCompact.length, rightCompact.length);
  return coverage >= 0.9 && lengthRatio >= 0.72;
}

/**
 * @param {string | undefined} current
 * @param {string | undefined} candidate
 * @param {number} currentCreatedAt
 * @param {number} candidateCreatedAt
 * @returns {string | undefined}
 */
export function latestResponseText(
  current,
  candidate,
  currentCreatedAt = 0,
  candidateCreatedAt = 0,
) {
  if (!chatTextKey(candidate)) return current;
  if (!chatTextKey(current)) return candidate;
  return Number(candidateCreatedAt) >= Number(currentCreatedAt) ? candidate : current;
}
