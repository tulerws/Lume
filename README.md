# Lume

Lume é uma sobreposição local e discreta para acompanhar agentes de IA no Windows e Linux. Ela reúne sessões em andamento, pedidos de permissão, conclusões e erros sem exigir que o usuário vigie cada terminal, IDE ou aba.

## Estado atual

O repositório contém a primeira base funcional:

- cápsula recolhida com contador de agentes;
- painel expandido com sessões separadas por agente e projeto;
- perfil de acesso individual por sessão;
- ações de permissão derivadas das capacidades do chat, sem botões globais presumidos;
- descarte do conteúdo sensível após a decisão;
- bandeja e inicialização automática preparadas no núcleo Tauri;
- tema claro e escuro;
- dados demonstrativos enquanto os adaptadores reais são conectados.

## Arquitetura

- **Tauri 2 + Rust:** janela, bandeja, autostart, descoberta de processos e IPC local.
- **Svelte 5 + TypeScript:** cápsula, lista de sessões, permissões e preferências.
- **Adaptadores:** Codex, Claude, Gemini, VS Code e extensão Chromium.
- **Persistência local:** o histórico guardará apenas agente, projeto, evento, horário e resumo sanitizado. Comandos, caminhos e payloads completos existirão somente enquanto a permissão estiver pendente.

Cada adaptador normaliza a configuração da sessão em um `PermissionProfile`. A interface renderiza apenas `availableActions`, permitindo que duas conversas do mesmo agente tenham políticas diferentes, como acesso total e somente leitura.

## Desenvolvimento

```bash
npm install
npm run check
npm run build
npm run tauri dev
```

O build desktop no Linux requer Rust e as dependências WebKitGTK do Tauri. Consulte os [pré-requisitos oficiais](https://v2.tauri.app/start/prerequisites/).

No Pop!_OS 24.04, instale os pacotes de desenvolvimento com:

```bash
sudo apt-get update
sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev libdbus-1-dev pkg-config
```

As decisões de produto já confirmadas estão em [`docs/PRODUCT.md`](docs/PRODUCT.md).

## Próximas entregas

1. Persistência local e preferências de monitor, tela cheia, som e retenção.
2. Adaptador Claude por hooks, incluindo resposta a `PermissionRequest`.
3. Adaptador Codex por hooks e app-server.
4. Adaptador Gemini por hooks e ACP.
5. Extensão para Chrome, Edge e Brave.
6. Integração com VS Code e retomada de sessões recentes.
7. Backend Wayland `layer-shell` para posicionamento confiável no GNOME/Pop!_OS.
