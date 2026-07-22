# Arquitetura

Este documento descreve como o Lume é construído por dentro: a topologia de processos,
o modelo de domínio compartilhado, os fluxos de dados e as invariantes que amarram tudo.
Para o propósito do produto veja [Produto](PRODUCT.md); para as garantias de dados veja
[Privacidade](PRIVACY.md).

## Visão geral

O Lume é uma sobreposição local (a "cápsula") que acompanha sessões de agentes de IA
(**Codex, Claude, Gemini**) já abertas ou iniciadas pelo próprio aplicativo. Tudo roda na
máquina: sem servidor remoto, conta ou telemetria.

- **Frontend:** SvelteKit 5 + TypeScript (Svelte 5 *runes*), `adapter-static`, SPA. Renderiza
  dentro de uma janela Tauri transparente e sem decoração — só a cápsula aparece.
- **Backend:** Rust + Tauri 2. Dependências centrais: `rusqlite` (SQLite embutido), `sysinfo`
  (descoberta de processos), `tungstenite` (WebSocket com o Codex), `notify`, e — no Linux —
  `gtk` + `libloading` para posicionamento por *layer-shell*; no Windows, `windows-sys`.
- **Companions:** uma extensão Chromium (Chrome/Edge/Brave) e uma extensão do VS Code.

## Um binário, vários papéis

O mesmo executável assume papéis diferentes conforme o primeiro argumento
(`src-tauri/src/main.rs`). Isso permite que os *hooks* dos agentes chamem o Lume em execução
sem um binário separado.

| Invocação            | Papel                                                                 |
| -------------------- | --------------------------------------------------------------------- |
| _(sem argumento)_    | Aplicativo Tauri / GUI (`lume_lib::run`)                               |
| `hook <provider>`    | Cliente de *hook* do agente → `adapters::run_hook`                     |
| `ingest`             | Lê JSONL do stdin e encaminha ao *event server* (`run_ingest_client`) |
| `terminal-run <p>`   | Processo aberto no terminal que executa o comando do agente           |

## Topologia de processos e portas

Todos os serviços escutam apenas em *loopback* (`127.0.0.1`).

| Porta   | Serviço                        | Arquivo                | Protocolo                             |
| ------- | ------------------------------ | ---------------------- | ------------------------------------- |
| `43119` | Entrada JSONL dos *hooks*      | `event_server.rs`      | TCP, uma linha JSON por conexão       |
| `43120` | Companion Chromium             | `browser_server.rs`    | HTTP/1.1, restrito a origem de extensão |
| `43130` | Codex *App Server* (sob demanda) | `codex_bridge.rs`     | WebSocket / JSON-RPC                   |
| `43131` | Ponte Lume ↔ Codex             | `codex_bridge.rs`      | WebSocket (proxy do App Server)       |

```
                         ┌──────────────── App Tauri (GUI) ────────────────┐
  Hooks dos agentes      │  Svelte (cápsula)  ⇅ invoke / emit ⇅  Rust       │
  (Claude/Codex/Gemini)  │                                                  │
        │ JSONL           │   AppState = sessões (memória) + SQLite          │
        ▼                 │             + canal de decisões (Condvar)         │
   :43119 event_server ───┼──► ingest(HookEvent) ──► lume://sessions-changed │
                          │                                                  │
   discovery (sysinfo/2s)─┼──► reconcile_processes ─────────────┘            │
                          │                                                  │
   Codex app-server ⇄ :43131 codex_bridge (proxy) ⇄ :43130 codex app-server │
                          │                                                  │
   Extensão Chromium ─────► :43120 browser_server (HTTP, só origem extensão) │
                          └──────────────────────────────────────────────────┘
```

## Modelo de domínio

O modelo de domínio é definido em `src-tauri/src/domain.rs` e **espelhado** em
`src/lib/domain.ts` (serde em `camelCase`). É o contrato que costura Rust ↔ frontend.

- `AgentKind` — `Codex` | `Claude` | `Gemini` | `Unknown`
- `SessionSource` — `Cli` | `Vscode` | `Web` | `Desktop`
- `SessionStatus` — `Running` | `PermissionRequired` | `WaitingForInput` | `Completed` | `Failed`
- `AccessMode` — `FullAccess` | `WorkspaceWrite` | `ReadOnly` | `Plan` | `Custom`
- `PermissionAction` — `AllowOnce` | `AllowSession` | `Deny` | `OpenSource`
- **`PermissionProfile`** — a política, **por sessão**:
  `{ mode, label, approvalPolicy, canRespondFromLume, availableActions }`.
- `AgentSession`, `PermissionRequest`, `HistoryEntry`, `Preferences`, `HookEvent`, `HookResponse`.

Duas regras nascem daqui e valem para todo o produto:

1. A interface **só desenha uma ação que esteja em `availableActions`**.
2. O Lume **só responde** por uma sessão quando `canRespondFromLume` é verdadeiro.

Cada conversa mantém seu próprio `PermissionProfile`; uma pode ter acesso amplo enquanto outra
do mesmo agente está em modo de planejamento ou somente leitura.

## Estado central: `AppState`

`src-tauri/src/state.rs` concentra a orquestração. `AppState` é clonável (tudo é `Arc`) e
compartilhado por todos os subsistemas via estado gerenciado do Tauri.

```rust
sessions:  Arc<Mutex<Vec<AgentSession>>>                     // verdade viva, em memória
store:     Arc<Mutex<Store>>                                  // SQLite (sanitizado)
decisions: Arc<(Mutex<HashMap<PermId, PermissionAction>>, Condvar)>  // canal de decisão
missing_process_scans: Arc<Mutex<HashMap<SessionId, u8>>>     // histerese de presença
```

A ordem de inicialização está em `lib.rs::run` → `setup`:
`AppState` → `CodexBridge` → `codex_sessions` (watcher) → `event_server` → `browser_server`
→ `terminal_windows` → `discovery` → guarda de tela cheia → configura o overlay e mostra a
janela → ícone de bandeja e *autostart*.

A superfície de comandos Tauri (o contrato chamável pelo frontend) também vive em `lib.rs`:
`list_sessions`, `resolve_permission`, `submit_prompt`, `terminate_session`, `list_history`,
`get_preferences`/`set_preferences`, `move_overlay`, `launch_session`, os comandos de janela de
terminal (`open`/`list`/`close`/`move`/`sync`/`resize`/`undock`) e os de integração
(`integration_statuses`, `configure_integration`, `vscode_status`, `configure_vscode`,
`reveal_browser_companion`).

## Ingestão: duas vias que convergem numa sessão

O estado de uma sessão pode chegar por dois caminhos que precisam ser reconciliados sem
duplicar cards.

### Via *push* — hooks

O agente dispara um *hook* → `event_server` (`:43119`) lê uma linha JSONL → `state.ingest`.
Sessões que vêm por aqui trazem um `native_session_id` (o identificador do chat no agente).
`adapters.rs` faz a **normalização**: traduz os nomes de eventos e as strings de modo de cada
agente para o `HookEventKind`/`AccessMode` do domínio e monta o `HookEvent`.

### Via *pull* — descoberta de processos

`discovery.rs` varre os processos com `sysinfo` a cada 2 s, classifica binários
`codex`/`claude`/`gemini` (por nome ou token do comando), sobe a árvore de ancestrais para
distinguir `Vscode` de `Cli`, e exclui o próprio PID e os processos da ponte do Codex. Cada
processo vira um `DiscoveredProcess` e entra em `reconcile_processes`.

Sessões descobertas apenas por processo são **provisórias** (`id = "process:<agente>:<pid>"`,
sem `native_session_id`) e aparecem como **`waiting_for_input`** — o Lume nunca finge saber que
o agente está "executando".

### Reconciliação

A parte mais delicada (e a mais testada) é fundir as duas visões:

- `ingest` **adota** a sessão provisória correspondente (mesmo agente + PID, ou mesmo contexto
  no VS Code) em vez de criar outra.
- Sessões são deduplicadas por `(agente, native_session_id)`, preferindo a que tem
  `can_respond_from_lume`.
- O processo hospedeiro do VS Code é escondido atrás dos chats nativos daquele agente.
- Depois de **3 varreduras** consecutivas sem enxergar o PID (histerese
  `missing_process_scans`, `PROCESS_MISSING_SCAN_LIMIT`), a sessão é marcada `completed` — o que
  evita piscar em lacunas transitórias de detecção.

O módulo de testes ao final de `state.rs` documenta cada uma dessas regras como casos
executáveis.

## Modelo de permissão

Este é o fluxo que dá sentido ao produto — uma ponte **síncrona** entre um cliente de *hook*
efêmero (outro processo) e a interface assíncrona.

1. O *hook* (ou a ponte do Codex) envia um `HookEvent` com `wait_for_decision: true`.
2. O cliente de *hook* **fica bloqueado na conexão TCP**, enquanto o servidor estaciona em
   `AppState::wait_for_decision` — um `Condvar` com *timeout* de 15 min sobre o mapa
   `decisions`, chaveado pelo `permission_id`.
3. A interface chama `resolve_permission`, que **valida** que a ação está em `availableActions`
   **e** que `canRespondFromLume`; apaga o `pending_permission` (privacidade); grava um resumo
   no histórico; insere a decisão no mapa; e faz `notify_all`.
4. O cliente de *hook* acorda, recebe um `HookResponse` e devolve a decisão ao agente no
   protocolo dele.

Se o processo **desaparece** enquanto a permissão está pendente, a reconciliação injeta um
`Deny` no canal para não deixar o agente travado.

### Quem pode responder pelo Lume

| Origem                         | Sessões existentes            | Resposta na cápsula                         |
| ------------------------------ | ----------------------------- | ------------------------------------------- |
| **Claude** (hooks)             | Hooks                         | **Sim** — `[AllowOnce, Deny]` (+`AllowSession` quando o CLI sugere) |
| **Codex aberto pelo Lume**     | App Server (`:43130`) via ponte (`:43131`) | **Sim** — respondido *inline* na ponte |
| Codex CLI/VS Code externos     | Processos + *watcher* JSONL   | Observação (`[OpenSource]`)                 |
| Gemini                         | Processos + hooks             | Observação                                  |
| ChatGPT/Claude/Gemini web      | Companion Chromium            | Abrir a aba correta (`OpenSource`)          |

Em `adapters.rs`, a resposta direta só é marcada para **Claude + `PermissionRequest`**. Para o
Claude, "permitir nesta sessão" reusa a sugestão do próprio CLI e só altera o destino para a
sessão atual.

## Integração profunda com o Codex

Três arquivos, com papéis distintos:

- **`codex_bridge.rs`** — o ciclo de vida do **App Server** (`codex app-server`, iniciado sob
  demanda em `:43130`, encerrado no `Drop`); o **proxy WebSocket em `:43131`** (o Codex aberto
  pelo Lume conecta de volta com `--remote`); o **JSON-RPC** de prompt
  (`initialize → thread/resume → turn/start`, monitorado até `turn/completed`); e a **resposta
  direta de aprovações** (`item/commandExecution` | `fileChange` | `permissions/requestApproval`
  → `PermissionRequest` → `wait_for_decision` → `accept`/`acceptForSession`/`decline`). As
  notificações do servidor viram `HookEvent`s.
- **`codex_sessions.rs`** — um *watcher* **somente-observação** dos logs JSONL do Codex externo
  ou aberto em terminal.
- **`launcher.rs`** — abre e retoma sessões conforme a preferência `launch_target`
  (`auto`/`terminal`/`vscode`).

Ambas as visões do Codex namespaceiam os identificadores como `codex-app-server:{threadId}`,
então a sessão observada e a sessão pela ponte **se fundem** numa só.

O `launcher` grava, no Linux, um *payload* JSON e um `.desktop` cujo `Exec` reinvoca o Lume como
`terminal-run`, então usa `gio launch`; no Windows usa `wt.exe` (com *fallback* para
`cmd.exe /K`). Para o VS Code, monta um *deep link* `vscode://tulerws.lume/session?payload=…`
aberto com `code --reuse-window`, tratado pela extensão. Para o Codex, o `--remote <ponte>` é
inserido **antes** do `resume`.

## Sobreposição e janelas

`overlay.rs` posiciona a cápsula:

- **Wayland:** carrega `libgtk-layer-shell` via `libloading`/`dlopen` (ponteiros de função
  `extern "C"` num `OnceLock`) e posiciona por **âncora + margem** (topo/esquerda), escolhendo o
  monitor por nome. "Esconder sob tela cheia" usa `set_layer(2)` (Top, fica abaixo de janelas em
  tela cheia) contra `set_layer(3)` (Overlay) quando `show_over_fullscreen` está ativo.
- **X11 / Windows:** uma *thread*-guarda (a cada ~900 ms) detecta se a janela em foco está em
  tela cheia (via `xprop` no X11, `MONITORINFO` no Win32) e alterna `always_on_top` de forma
  idempotente.
- A posição arrastada é persistida em `overlay_x` / `overlay_y`.

`terminal_windows.rs` implementa o **Whiteboard**: cada sessão abre uma `WebviewWindow` Tauri
flutuante (não um terminal nativo), com rótulo `terminal-<hash>`, carregando a mesma SPA — que
renderiza `TerminalWindow.svelte` quando o rótulo começa com `terminal-`. Janelas próximas
podem **acoplar**: ao soltar a menos de ~34 px de outra, ocorre um *snap* de bordas e os grupos
se fundem, passando a se mover em conjunto.

## Companions

### Navegador (`browser_server.rs` + `extensions/chromium/`)

Servidor HTTP/1.1 escrito à mão em `:43120`, com rotas `POST /events`, `GET /health` e
`OPTIONS`, corpo limitado a 64 KB, que **só aceita `Origin` de extensão** (`chrome-extension://`).
A extensão (MV3) infere provedor, estado e título por heurística de DOM e envia apenas **agente,
estado, título sanitizado, origem e um hash FNV-1a do caminho** (`web:{provider}:{hash}`). O
perfil é travado em `OpenSource`.

O *composer* nunca empurra texto para o navegador: o Lume enfileira o prompt em
`BrowserControl` e a extensão o retira no próximo *poll* (`{ focus, prompt }`), digitando na aba
selecionada. O texto permanece **local à aba** e não entra no histórico do Lume.

### VS Code (`extensions/vscode/`)

Um *handler* de URI `vscode://…/session?payload=` que valida `agent ∈ {codex, claude, gemini}`
e o `cwd`, então abre um terminal integrado rodando o agente. Não reporta nada de volta. É
empacotado em `.vsix` por `scripts/package-vscode.mjs` e embarcado como recurso do Tauri.

## Frontend

Uma única rota (`src/routes/+page.svelte`) que serve dois modos de janela: a **cápsula** e a
janela de **terminal** (`TerminalWindow.svelte`, escolhida quando o rótulo começa com
`terminal-`). O estado usa *runes* (`$state`/`$derived`/`$effect`). `isTauri` protege as
chamadas nativas; fora do Tauri a interface hidrata de `demo.ts` para renderizar isolada.

`src/lib/lume.ts` embrulha cada `invoke(...)` com *fallback* de demonstração e assina os eventos
`lume://sessions-changed` e `lume://terminal-windows-changed` (com *poll* de 5 s como reserva).
O **mascote** (`LumeMascot.svelte`) reduz todas as sessões a um único estado visual
(permissão > falha > *running* > espera > concluído > ocioso), mapeado para cor e animação. A
altura da cápsula é dinâmica (`ResizeObserver`). `src/lib/i18n.ts` traduz os rótulos de status
em pt-BR emitidos pelo backend para inglês na exibição.

Para um guia introdutório da interface — voltado a quem nunca usou Svelte, com o mapa de
"quero mudar X, edito onde" — veja [Interface](FRONTEND.md).

## Persistência e privacidade

O banco SQLite (`store.rs`) fica no diretório de dados do aplicativo e guarda apenas dados
sanitizados. As invariantes são aplicadas **no ponto de gravação e no boot**, não como um filtro
posterior:

- `save_session` **remove `pending_permission`, `working_directory` e `last_response` antes de
  toda gravação**. Comando, caminho e *payload* de uma permissão existem **só em memória**
  enquanto a decisão está pendente.
- Na inicialização, `AppState::new` rebaixa qualquer status ativo obsoleto para `Completed`
  ("Aguardando redetecção"), executa `VACUUM` + `wal_checkpoint(TRUNCATE)` (`scrub_deleted_content`)
  para não deixar rastros em WAL, e purga o histórico além da retenção (padrão **30 dias**).
- `PRAGMA secure_delete = ON`. O histórico guarda só resumos ("Tarefa finalizada", "Permissão
  concedida/recusada"). Uma decisão é vinculada ao par `(permission_id, session_id)`; respostas
  fora desse par são recusadas.

## Build e release

- **CI** (`.github/workflows/ci.yml`, Ubuntu + Windows): `npm run check` → `npm run build:vscode`
  → `npm run build` → `cargo test`.
- **Instaladores** (`.github/workflows/installers.yml`, em *tag* `v*` ou manual): usa
  `tauri-action` para gerar `.deb` + AppImage (Linux) e NSIS (Windows), cria a GitHub Release e
  publica o `latest.json` consumido pelo atualizador. Requer o *secret*
  `TAURI_SIGNING_PRIVATE_KEY`.
- O número de versão precisa ser mantido em sincronia entre `package.json`,
  `src-tauri/Cargo.toml` e `src-tauri/tauri.conf.json`.
- O `.deb` depende de `libgtk-layer-shell0`; no AppImage em Wayland esse pacote precisa estar
  instalado no sistema para o posicionamento nativo por monitor.

## Mapa de arquivos

| Arquivo                          | Responsabilidade                                                    |
| -------------------------------- | ------------------------------------------------------------------- |
| `src-tauri/src/main.rs`          | Despacho por `argv` entre os papéis do binário                      |
| `src-tauri/src/lib.rs`           | Comandos Tauri e sequência de inicialização                         |
| `src-tauri/src/domain.rs`        | Modelo de domínio (espelhado em `src/lib/domain.ts`)                |
| `src-tauri/src/state.rs`         | `AppState`, ingestão, reconciliação e canal de decisões             |
| `src-tauri/src/store.rs`         | Persistência SQLite sanitizada                                      |
| `src-tauri/src/event_server.rs`  | Entrada JSONL dos *hooks* (`:43119`)                                |
| `src-tauri/src/discovery.rs`     | Descoberta de processos por `sysinfo`                               |
| `src-tauri/src/adapters.rs`      | Normalização dos *hooks* de cada agente                             |
| `src-tauri/src/integrations.rs`  | Instalação de *hooks* preservando a configuração existente          |
| `src-tauri/src/executables.rs`   | Localização dos binários dos agentes                                |
| `src-tauri/src/codex_bridge.rs`  | App Server do Codex, ponte `:43131` e resposta de aprovações        |
| `src-tauri/src/codex_sessions.rs`| *Watcher* de logs do Codex externo                                  |
| `src-tauri/src/launcher.rs`      | Abertura e retomada de sessões (terminal / VS Code)                 |
| `src-tauri/src/overlay.rs`       | Posicionamento da cápsula e guarda de tela cheia                    |
| `src-tauri/src/terminal_windows.rs` | Whiteboard: janelas flutuantes e acoplamento                     |
| `src-tauri/src/browser_server.rs`| Servidor do Companion Chromium (`:43120`)                           |
| `src/routes/+page.svelte`        | Interface principal (cápsula e navegação)                           |
| `src/lib/lume.ts`                | Ponte de comandos e eventos com o Rust                              |
| `src/lib/i18n.ts`                | Tradução dos rótulos pt-BR → inglês                                 |
| `extensions/chromium/`           | Extensão do navegador (Companion)                                   |
| `extensions/vscode/`             | Extensão do VS Code                                                 |
