const vscode = require("vscode");

const agents = new Set(["codex", "claude", "gemini"]);

function activate(context) {
  const openSession = (request) => {
    if (!request || !agents.has(request.agent) || typeof request.cwd !== "string") {
      void vscode.window.showErrorMessage("O Lume recebeu uma sessão inválida.");
      return;
    }
    const args = Array.isArray(request.args)
      ? request.args.filter((value) => typeof value === "string").slice(0, 8)
      : [];
    const terminal = vscode.window.createTerminal({
      name: `Lume · ${request.agent[0].toUpperCase()}${request.agent.slice(1)}`,
      cwd: vscode.Uri.file(request.cwd),
      shellPath: request.agent,
      shellArgs: args,
      iconPath: new vscode.ThemeIcon("sparkle"),
    });
    terminal.show();
  };

  context.subscriptions.push(
    vscode.window.registerUriHandler({
      handleUri(uri) {
        if (uri.path !== "/session") return;
        try {
          const params = new URLSearchParams(uri.query);
          openSession(JSON.parse(params.get("payload") ?? ""));
        } catch {
          void vscode.window.showErrorMessage("Não foi possível abrir a sessão enviada pelo Lume.");
        }
      },
    }),
    vscode.commands.registerCommand("lume.openSession", () => {
      void vscode.window.showInformationMessage(
        "Abra uma sessão pela cápsula do Lume para escolher agente e projeto.",
      );
    }),
  );
}

function deactivate() {}

module.exports = { activate, deactivate };
