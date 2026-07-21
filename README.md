# Lume

Lume é uma sobreposição local e discreta para acompanhar agentes de IA no Windows e Linux. A cápsula fica no topo da tela, sinaliza trabalho, permissões, conclusão e erro, e expande para uma lista contínua de sessões — sem painéis cheios de cards.

## O que já funciona

- descoberta de processos Codex, Claude e Gemini já abertos;
- hooks preservando as configurações existentes de cada agente;
- permissão direta do Claude e de sessões Codex abertas pelo Lume;
- perfil de acesso e ações válidas por conversa;
- abertura e retomada no terminal ou no terminal integrado do VS Code;
- Companion para VS Code e para Chrome, Edge e Brave;
- histórico local sanitizado, sons opcionais, bandeja e autostart;
- monitor configurável e sobreposição Wayland por `gtk-layer-shell`;
- comportamento padrão abaixo de vídeos e jogos em tela cheia;
- tema claro/escuro, movimento reduzido e microinterações fluidas.

## Rodar em desenvolvimento

Requisitos: Node.js 22+, Rust estável e as dependências do Tauri.

No Pop!_OS/Ubuntu:

```bash
sudo apt-get update
sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev libgtk-layer-shell0 build-essential curl wget file libssl-dev libayatana-appindicator3-dev librsvg2-dev libdbus-1-dev pkg-config
```

Depois:

```bash
npm install
npm run check
npm run tauri dev
```

O Lume aparece no topo do monitor principal e também cria um ícone na bandeja. Em **Ajustes**, conecte os agentes instalados, o VS Code e abra a pasta do Companion web.

## Conectar as origens

| Origem | Sessões existentes | Permissão na cápsula |
| --- | --- | --- |
| Claude CLI | Hooks | Sim |
| Codex CLI/VS Code externos | Processos + hooks | Observação |
| Codex aberto pelo Lume | App Server local | Sim |
| Gemini CLI | Processos + hooks | Observação |
| ChatGPT, Claude e Gemini web | Companion Chromium | Abrir a aba correta |

O Lume só mostra botões que a sessão atual suporta. No Gemini e em sessões externas do Codex, a origem continua responsável pela decisão; o Lume não simula uma autorização que a integração não oferece.

Depois de conectar o Codex pela primeira vez, abra `/hooks` no próprio Codex e confie no hook **Lume**. O Codex exige essa confirmação para hooks locais novos ou alterados.

Para instalar o Companion web, abra **Ajustes → Navegadores → Abrir pasta**, acesse `chrome://extensions` (ou a página equivalente do Edge/Brave), ative o modo de desenvolvedor e carregue a pasta sem compactação. O Companion envia apenas agente, estado, título sanitizado, origem e um hash local do caminho.

## Build e instaladores

```bash
npm run tauri build
```

No Linux, os pacotes ficam em `src-tauri/target/release/bundle`. O workflow **Instaladores** gera `.deb`, AppImage e instalador NSIS para Windows, manualmente ou em tags `v*`.

O `.deb` instala a dependência `libgtk-layer-shell0`. Ao usar o AppImage em Wayland, instale esse pacote no sistema para obter posicionamento nativo por monitor e o comportamento correto diante de tela cheia.

## Privacidade

Tudo permanece na máquina. Os serviços escutam somente em `127.0.0.1:43119`, `127.0.0.1:43120`, `127.0.0.1:43130` e `127.0.0.1:43131`. Comandos, caminhos e payloads de uma permissão existem em memória apenas enquanto a decisão está pendente; o SQLite recebe somente a sessão sanitizada e resumos do histórico.

Mais detalhes em [Produto](docs/PRODUCT.md) e [Privacidade](docs/PRIVACY.md).
