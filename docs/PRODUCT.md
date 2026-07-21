# Produto

## Objetivo

Lume acompanha sessões de agentes de IA que já estejam abertas ou que tenham sido iniciadas pelo próprio aplicativo. O produto é totalmente local e não envia histórico, comandos, caminhos ou conteúdo de projetos para um serviço próprio.

## Experiência principal

- A cápsula inicia recolhida no topo do monitor principal.
- O usuário pode escolher outro monitor nas preferências.
- Por padrão, a cápsula não aparece sobre vídeos ou jogos em tela cheia.
- Uma preferência permite manter a sobreposição visível em tela cheia.
- Ao expandir, a cápsula mostra todos os agentes, cada sessão, projeto, origem e estado.
- Sons sutis de conclusão e erro podem ser desativados.
- A bandeja permite mostrar ou ocultar o painel e sair.
- Sessões podem ser abertas ou retomadas no terminal ou VS Code habitual.
- O Whiteboard abre cada sessão em seu próprio mini terminal; janelas próximas podem ser acopladas e movidas como um conjunto.

## Estados normalizados

- `running`: o agente está trabalhando.
- `permission_required`: existe uma decisão de segurança pendente.
- `waiting_for_input`: o agente aguarda uma resposta que não é uma permissão.
- `completed`: a tarefa terminou normalmente.
- `failed`: houve erro ou uma ação necessária falhou.

## Permissões por sessão

Cada conversa mantém seu próprio `PermissionProfile`. O perfil é obtido do agente e inclui:

- modo de acesso atual;
- política de aprovação atual;
- se o Lume pode responder por aquela integração;
- ações válidas para a solicitação atual.

A interface nunca cria um botão apenas porque outro chat do mesmo agente o oferece. Uma sessão pode ter acesso total enquanto outra está em modo de planejamento ou somente leitura.

O conteúdo detalhado da permissão permanece apenas na memória enquanto a decisão está pendente. Depois de permitir ou recusar, o histórico mantém um resumo sanitizado, sem comando completo, payload, caminho absoluto ou segredo.

## Plataformas iniciais

- Windows 10 e 11.
- Pop!_OS e outras distribuições Linux modernas.
- X11/XWayland pela janela Tauri.
- Wayland nativo por um backend de posicionamento `layer-shell` quando o compositor oferecer o protocolo.

## Integrações iniciais

- Codex CLI e extensão do VS Code.
- Claude CLI e extensão do VS Code.
- Gemini CLI.
- VS Code como IDE inicial.
- Chrome, Edge e Brave por uma extensão Chromium local.

Integrações profundas respondem permissões diretamente. Integrações somente observáveis mostram o pedido e levam o usuário à origem, sem simular suporte inexistente.
