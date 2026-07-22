# Lume

Lume é uma sobreposição local e discreta para acompanhar agentes de IA no Windows e Linux. A cápsula fica no topo da tela, sinaliza trabalho, permissões, conclusão e erro, e expande para uma lista contínua de sessões.

## O que já funciona

- descoberta de processos Codex, Claude e Gemini já abertos;
- hooks preservando as configurações existentes de cada agente;
- permissão direta do Claude e de sessões Codex abertas pelo Lume;
- perfil de acesso e ações válidas por conversa;
- abertura e retomada no terminal ou no terminal integrado do VS Code;
- Companion para VS Code e para Chrome, Edge e Brave;
- histórico local sanitizado, sons opcionais, bandeja e autostart;
- monitor configurável e sobreposição Wayland por `gtk-layer-shell`;
- cápsula arrastável com posição salva entre reinicializações;
- Whiteboard com um mini terminal flutuante por sessão e acoplamento entre janelas;
- layouts nomeados do Whiteboard, restaurados para as sessões abertas correspondentes;
- perfis por projeto com destino, monitor, posição, permissões, layout e agentes preferidos;
- respostas finais com arquivos/verificações reportados e opção explícita de salvar como nota;
- detectores externos instaláveis por manifesto JSON, sem executar código de terceiros;
- paleta de comandos global em `Ctrl+Shift+Space`;
- continuação por prompt no Codex aberto pelo Lume e nos chats web conectados;
- comportamento padrão abaixo de vídeos e jogos em tela cheia;

## Rodar em desenvolvimento

Requisitos: Node.js 22+, Rust estável e as dependências do Tauri.

No Pop!_OS/Ubuntu:

```bash
sudo apt-get update
sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev libgtk-layer-shell0 build-essential curl wget file libssl-dev libayatana-appindicator3-dev librsvg2-dev libdbus-1-dev pkg-config
```

No Fedora:

```bash
sudo dnf install -y webkit2gtk4.1-devel gtk3-devel gtk-layer-shell openssl-devel libappindicator-gtk3-devel librsvg2-devel dbus-devel gcc gcc-c++ make curl wget file pkgconf-pkg-config
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

O Lume só mostra botões que a sessão atual suporta. No Gemini e em sessões externas do Codex, a origem continua responsável pela decisão; o Lume não simula uma autorização que a integração não oferece. O composer envia diretamente para sessões Codex abertas pelo Lume e para páginas conectadas pelo Companion. Terminais externos permanecem somente para acompanhamento.

Depois de conectar o Codex pela primeira vez, abra `/hooks` no próprio Codex e confie no hook **Lume**. O Codex exige essa confirmação para hooks locais novos ou alterados.

Para instalar o Companion web, abra **Ajustes → Navegadores → Abrir pasta**, acesse `chrome://extensions` (ou a página equivalente do Edge/Brave), ative o modo de desenvolvedor e carregue a pasta sem compactação. O Companion envia apenas agente, estado, título sanitizado, origem e um hash local do caminho. Quando você usa o composer, o texto segue somente pela conexão local até a aba selecionada e não entra no histórico do Lume.

## Build e instaladores

```bash
npm run tauri build
```

No Linux, os pacotes ficam em `src-tauri/target/release/bundle` (o `.rpm` em `bundle/rpm`). Para gerar apenas o pacote do Fedora, use `npm run tauri build -- --bundles rpm`. Sem a chave de assinatura configurada localmente, o comando termina com um erro do passo de assinatura do updater, mas o `.rpm` já foi gravado antes disso; instale-o com `sudo dnf install ./src-tauri/target/release/bundle/rpm/Lume-*.x86_64.rpm`. O workflow **Instaladores** gera `.deb`, `.rpm`, AppImage e instalador NSIS para Windows, cria a GitHub Release e publica o `latest.json` usado pelo atualizador.

Antes da próxima release, cadastre a chave privada de assinatura em **Settings → Secrets and variables → Actions** com o nome `TAURI_SIGNING_PRIVATE_KEY`. A chave pública já fica no aplicativo; a privada nunca deve entrar no repositório. Em cada nova versão, atualize o número em `package.json`, `src-tauri/Cargo.toml` e `src-tauri/tauri.conf.json`, então execute o workflow ou publique uma tag `v*`.

A versão 0.3.0 precisa ser instalada manualmente uma vez porque as versões anteriores ainda não contêm o atualizador. Depois disso, o Lume verifica novas versões ao iniciar e oferece a instalação em **Ajustes → Sobre**. No Linux, o AppImage é substituído no próprio local e o `.deb` pode pedir a autenticação do sistema durante a instalação.

O `.deb` instala a dependência `libgtk-layer-shell0`. Ao usar o AppImage em Wayland, instale esse pacote no sistema para obter posicionamento nativo por monitor e o comportamento correto diante de tela cheia.

No Wayland, o renderizador DMABUF do WebKitGTK pode derrubar a conexão com `Error 71 (Protocol error)` antes de a janela abrir, em algumas combinações de compositor e GPU (por exemplo, KWin com NVIDIA), por causa da sincronização explícita. Por isso o Lume desativa esse renderizador automaticamente **apenas no Wayland**; no X11 o DMABUF continua ativo. Quem já define `WEBKIT_DISABLE_DMABUF_RENDERER` no ambiente mantém sua própria escolha.

## Detectores externos

Em **Ajustes → Detectores externos**, instale um manifesto JSON para acompanhar outra CLI. O formato está em [`docs/external-plugin.example.json`](docs/external-plugin.example.json). O manifesto só declara nomes e tokens de processo usados na detecção; ele não carrega bibliotecas nem executa comandos no Lume. Alterações passam a valer na próxima varredura, sem reiniciar o aplicativo.

## Privacidade

Tudo permanece na máquina. Os serviços escutam somente em `127.0.0.1:43119`, `127.0.0.1:43120`, `127.0.0.1:43130` e `127.0.0.1:43131`. Sessões e respostas finais ficam apenas em memória. O SQLite recebe preferências, resumos sanitizados do histórico e somente as respostas que o usuário escolher explicitamente salvar como nota.

Mais detalhes em [Produto](docs/PRODUCT.md), [Privacidade](docs/PRIVACY.md), [Arquitetura](docs/ARCHITECTURE.md) e [Interface](docs/FRONTEND.md).
