import type { Preferences } from "$lib/domain";

export type Language = Preferences["language"];

export function localize(language: Language, english: string, portuguese: string) {
  return language === "pt-BR" ? portuguese : english;
}

const englishDisplayText = new Map<string, string>([
  ["Aguardando ação", "Waiting for input"],
  ["Esperando ação", "Waiting for input"],
  ["Aguardando sua resposta", "Waiting for your input"],
  ["Aguardando permissão", "Waiting for permission"],
  ["Executando", "Running"],
  ["Executando no VS Code", "Running in VS Code"],
  ["Finalizado", "Completed"],
  ["Falhou", "Failed"],
  ["Erro", "Error"],
  ["Encerrado", "Stopped"],
  ["Tarefa encerrada com erro", "Task failed"],
  ["Prompt enviado pelo Lume", "Prompt sent by Lume"],
  ["Continuando a tarefa", "Continuing task"],
  ["Permissão recusada", "Permission denied"],
  ["Sessão detectada", "Session detected"],
  ["Finalizado pelo hook", "Completed by hook"],
  ["Permissões da sessão", "Session permissions"],
  ["Gerenciada na origem", "Managed at source"],
  ["Ações disponíveis conforme o hook", "Actions available through the hook"],
  ["Monitoramento local", "Local monitoring"],
  ["A resposta depende da origem", "Response depends on the source"],
  ["Integração direta", "Direct integration"],
  ["Perguntar", "Ask"],
  ["Somente observação", "Monitoring only"],
  ["Abrir origem", "Open source"],
  ["Acesso amplo", "Full access"],
  ["A sessão normalmente não solicita confirmação", "The session normally does not request confirmation"],
  ["Modo de planejamento", "Plan mode"],
  ["Alterações não são permitidas", "Changes are not allowed"],
  ["Edições permitidas", "Edits allowed"],
  ["Outras ações ainda podem pedir confirmação", "Other actions may still request confirmation"],
  ["Somente leitura", "Read only"],
  ["Alterações exigem permissão", "Changes require permission"],
  ["Segue a configuração desta conversa", "Uses this conversation's configuration"],
  ["Pede confirmação fora do workspace", "Requests confirmation outside the workspace"],
  ["Confirma alterações e comandos", "Confirms changes and commands"],
  ["Executar a suíte de testes do projeto", "Run the project's test suite"],
  ["Sessão finalizada", "Session completed"],
  ["Permissão concedida uma vez", "Permission allowed once"],
  ["Monitoramento e decisões conectados", "Monitoring and decisions connected"],
  ["Necessário para abrir sessões no editor", "Required to open sessions in the editor"],
  ["CLI não encontrada", "CLI not found"],
  ["Hook conectado; /hooks está disponível no Codex CLI", "Hook connected; /hooks is available in Codex CLI"],
  ["Monitoramento conectado", "Monitoring connected"],
  ["Disponível para conectar", "Available to connect"],
  ["Companion conectado", "Companion connected"],
  ["Companion disponível", "Companion available"],
  ["VS Code não encontrado", "VS Code not found"],
  ["Verificando…", "Checking…"],
  ["Versão", "Version"],
  ["Monitoramento", "Monitoring"],
  ["Último evento", "Last event"],
  ["Hook do Lume ainda não conectado", "Lume hook is not connected yet"],
  ["Nenhum evento recebido nesta execução", "No event received in this run"],
  ["Não foi possível consultar a versão", "Could not read the version"],
]);

export function displayText(language: Language, value: string) {
  if (language === "pt-BR") return value;
  const exact = englishDisplayText.get(value);
  if (exact) return exact;
  if (value.startsWith("Finalizado há ")) return value.replace("Finalizado há ", "Completed ").replace(" min", " min ago");
  if (value.endsWith(" quer executar uma ação")) {
    return `${value.slice(0, -" quer executar uma ação".length)} wants to perform an action`;
  }
  if (/^\d+ eventos configurados$/.test(value)) {
    return value.replace(" eventos configurados", " events configured");
  }
  if (value.endsWith(" não encontrado")) {
    return `${value.slice(0, -" não encontrado".length)} not found`;
  }
  return value;
}
