import type { SessionActivity } from "$lib/domain";
import type { Language } from "$lib/i18n";

export type ActivityCategory = "edit" | "read" | "search" | "test" | "command" | "tool" | "plan";

function firstLine(value?: string): string {
  return String(value ?? "").split(/\n/, 1)[0].trim();
}

function normalizedToolTitle(value: string): string {
  return value
    .replace(/^functions\s*[·:]\s*/i, "")
    .replace(/^functions[.:/]/i, "")
    .trim();
}

export function activityCategory(activity: SessionActivity): ActivityCategory {
  const title = normalizedToolTitle(activity.title).toLowerCase();
  const detail = firstLine(activity.detail).toLowerCase();
  const searchable = `${title} ${detail}`;
  if (activity.kind === "plan" || /(?:^|[._/-])update_plan$/.test(title)) return "plan";
  if (activity.kind === "file" || /apply_patch|patch|edit(?:ed)?\s+file/.test(title)) return "edit";
  if (activity.kind === "test") return "test";
  if (/web.?search|search_query|pesquisa na web/.test(searchable)) return "search";
  if (/\b(?:rg|grep|find|fd)\b/.test(detail) || /search|searched|buscar|procurar/.test(title)) return "search";
  if (/view_image|read|inspect|open file|imagem inspecionada/.test(title)) return "read";
  if (/^\s*(?:cat|sed\s+-n|head|tail|ls|stat)\b/.test(detail)) return "read";
  if (
    activity.kind === "command"
    || /^(?:exec|exec_command|shell|terminal)$/.test(title)
    || /functions[.:/]exec/.test(activity.title.toLowerCase())
  ) {
    return /\b(?:test|check|lint|build|pytest|vitest|jest)\b/.test(detail) ? "test" : "command";
  }
  return "tool";
}

export function isPresentableTraceActivity(activity: SessionActivity): boolean {
  if (["prompt", "message", "analysis", "queued_prompt", "plan", "plan_document"].includes(activity.kind)) return false;
  const title = normalizedToolTitle(activity.title).toLowerCase();
  return !/^(?:create_goal|get_goal|update_goal|update_plan)$/.test(title);
}

export function isHiddenAgentActivity(activity: SessionActivity): boolean {
  if (["plan", "plan_document", "queued_prompt"].includes(activity.kind)) return true;
  const title = normalizedToolTitle(activity.title).toLowerCase();
  return /^(?:create_goal|get_goal|update_goal|update_plan)$/.test(title);
}

export function needsUserAuthorization(text?: string): boolean {
  const normalized = String(text ?? "")
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase();
  const compact = normalized.replace(/\s+/g, " ").trim();
  const directAuthorizationQuestions = [
    /\bvoce\s+(?:me\s+)?autoriza\b[^?\n]{0,240}\?/,
    /\bdo\s+you\s+(?:authorize|allow|approve|permit)\b[^?\n]{0,240}\?/,
    /\b(?:may|can)\s+i\s+(?:proceed|continue|run|execute|change|modify|delete|access|open|send|install)\b[^?\n]{0,240}\?/,
    /\b(?:me\s+)?autorizas\b[^?\n]{0,240}\?/,
    /\b(?:puedo|puedo\s+yo)\s+(?:proceder|continuar|ejecutar|cambiar|modificar|eliminar|acceder|abrir|enviar|instalar)\b[^?\n]{0,240}\?/,
    /\b(?:m['’])?autorisez-vous\b[^?\n]{0,240}\?/,
    /\best-ce\s+que\s+vous\s+(?:m['’])?autorisez\b[^?\n]{0,240}\?/,
    /\b(?:erlauben|genehmigen)\s+sie\b[^?\n]{0,240}\?/,
    /\bmi\s+autorizzi\b[^?\n]{0,240}\?/,
    /\bposso\s+(?:procedere|continuare|eseguire|modificare|eliminare|accedere|aprire|inviare|installare)\b[^?\n]{0,240}\?/,
  ];
  const explicitAuthorizationRetries = [
    /(?:^|[.!?]\s+)(?:por favor[,\s]+)?autorize\s+(?:novamente|de novo)\b.{0,240}\b(?:para|pra)\b/,
    /(?:^|[.!?]\s+)(?:please\s+)?(?:authorize|approve|allow|grant)\b.{0,160}\b(?:again|once more)\b/,
    /(?:^|[.!?]\s+)(?:por favor[,\s]+)?(?:autoriza\s+de\s+nuevo|vuelve\s+a\s+autorizar)\b.{0,240}\bpara\b/,
    /(?:^|[.!?]\s+)(?:veuillez\s+)?autorisez\b.{0,120}\b(?:a\s+nouveau|de\s+nouveau)\b/,
    /(?:^|[.!?]\s+)(?:bitte\s+)?(?:autorisieren|genehmigen)\b.{0,120}\berneut\b/,
    /(?:^|[.!?]\s+)(?:per\s+favore[,\s]+)?autorizza\s+(?:nuovamente|di\s+nuovo)\b/,
  ];
  return directAuthorizationQuestions.some((pattern) => pattern.test(normalized))
    || explicitAuthorizationRetries.some((pattern) => pattern.test(compact));
}

export function activityPreview(activity: SessionActivity): string {
  const line = firstLine(activity.detail)
    .replace(/^\{\s*"cmd"\s*:\s*"/i, "")
    .replace(/"\s*\}\s*$/, "");
  return line.length > 150 ? `${line.slice(0, 147)}…` : line;
}

export function activityDisplayTitle(activity: SessionActivity, language: Language): string {
  const pt = language === "pt-BR";
  const category = activityCategory(activity);
  if (category === "edit") {
    const count = new Set(activity.files).size;
    if (count > 0) return pt ? `${count} arquivo${count === 1 ? " alterado" : "s alterados"}` : `${count} file${count === 1 ? " edited" : "s edited"}`;
    return pt ? "Arquivos alterados" : "Edited files";
  }
  if (category === "read") return pt ? "Contexto inspecionado" : "Inspected context";
  if (category === "search") return pt ? "Busca no projeto" : "Searched the project";
  if (category === "test") return pt ? "Validação executada" : "Ran a check";
  if (category === "command") return pt ? "Comando executado" : "Ran a command";
  const title = normalizedToolTitle(activity.title);
  return title || (pt ? "Ferramenta utilizada" : "Used a tool");
}

function phrase(language: Language, category: ActivityCategory, count: number, fileCount: number): string {
  const pt = language === "pt-BR";
  if (category === "edit") {
    const total = fileCount || count;
    return pt ? `${total} arquivo${total === 1 ? " alterado" : "s alterados"}` : `${total} file${total === 1 ? " edited" : "s edited"}`;
  }
  if (category === "read") return pt ? "contexto lido" : "read context";
  if (category === "search") return pt ? "projeto pesquisado" : "searched the project";
  if (category === "test") return pt ? `${count} validaç${count === 1 ? "ão" : "ões"}` : `${count} check${count === 1 ? "" : "s"}`;
  if (category === "command") return pt ? `${count} comando${count === 1 ? "" : "s"}` : `${count} command${count === 1 ? "" : "s"}`;
  return pt ? `${count} ferramenta${count === 1 ? "" : "s"}` : `${count} tool${count === 1 ? "" : "s"}`;
}

export function activityGroupSummary(activities: SessionActivity[], language: Language): string {
  const counts = new Map<ActivityCategory, number>();
  const files = new Set<string>();
  for (const activity of activities) {
    const category = activityCategory(activity);
    if (category === "plan") continue;
    counts.set(category, (counts.get(category) ?? 0) + 1);
    if (category === "edit") activity.files.forEach((file) => files.add(file));
  }
  const order: ActivityCategory[] = ["edit", "read", "search", "test", "command", "tool"];
  const parts = order.flatMap((category) => {
    const count = counts.get(category) ?? 0;
    return count ? [phrase(language, category, count, files.size)] : [];
  });
  if (parts.length === 0) return language === "pt-BR" ? "Atividade do agente" : "Agent activity";
  const text = parts.join(", ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}
