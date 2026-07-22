# Guia da interface (frontend)

Este documento mapeia onde e como a interface do Lume é construída. Ele é escrito para
quem conhece HTML/JS mas nunca trabalhou com **Svelte** — o objetivo é que, ao final,
você saiba exatamente onde mexer para mudar qualquer coisa que o usuário vê. Para a
visão geral do sistema veja [Arquitetura](ARCHITECTURE.md).

## Onde a UI mora

Toda a interface fica na pasta `src/` (o `src-tauri/` é o backend Rust). Quase tudo que o
usuário vê está em **um único arquivo**: `src/routes/+page.svelte`.

```
src/
├── routes/
│   ├── +page.svelte      A TELA. A cápsula e suas quatro vistas principais.
│   ├── +layout.svelte    "Moldura" em volta de toda página (aqui só carrega o CSS).
│   └── +layout.ts        Config: ssr = false (roda 100% no webview, sem servidor).
├── lib/
│   ├── LumeMascot.svelte  O mascote (componente reutilizável).
│   ├── BrandIcon.svelte   Ícones de Codex/Claude/Gemini/navegadores.
│   ├── LumeLogo.svelte    O logo.
│   ├── TerminalWindow.svelte  A janela flutuante do Whiteboard.
│   ├── domain.ts         Os TIPOS (espelho de src-tauri/src/domain.rs).
│   ├── lume.ts           As CHAMADAS ao Rust (invoke).
│   ├── i18n.ts           Traduções pt/en.
│   └── demo.ts           Dados falsos para desenvolver sem agente rodando.
├── app.html              Esqueleto HTML onde tudo é injetado.
└── app.css               Estilos globais.
```

Regra mental: **`routes/` = telas**, **`lib/` = peças reutilizáveis e lógica**.

## Como uma "página web" vira uma janela de desktop

O Svelte gera um site estático e o Tauri o carrega dentro do webview:

- `app.html` é o HTML base; `%sveltekit.body%` é onde o app é injetado.
- `+layout.ts` define `export const ssr = false` → não há servidor; é uma SPA que roda
  inteira dentro da janela.
- `svelte.config.js` usa `adapter-static` com `fallback: "index.html"` → o build vira
  arquivos estáticos na pasta `build/`.
- `tauri.conf.json` aponta `frontendDist: "../build"` → o Tauri abre esses arquivos no
  webview transparente.

Ou seja: **a "página" é a interface do desktop.** Você desenvolve como se fosse um site,
e o Tauri o embrulha numa janela.

## Anatomia de um arquivo `.svelte`

Um `.svelte` tem três seções. `src/lib/LumeLogo.svelte` (24 linhas) é o exemplo mínimo:

```svelte
<script lang="ts">
  let { size = 28 }: { size?: number } = $props();   // (1) lógica + entradas
</script>

<span class="lume-logo" style:--logo-size={`${size}px`}>
  <img src="/lume.svg" alt="" />                       <!-- (2) markup (HTML) -->
</span>

<style>
  .lume-logo { width: var(--logo-size); ... }          /* (3) CSS escopado */
</style>
```

1. `<script>` — JavaScript/TypeScript: variáveis, funções e as **entradas** do componente.
2. Markup — HTML normal, com `{ }` para as partes dinâmicas.
3. `<style>` — CSS que vale **só para este componente** (escopado automaticamente). Aquele
   `.lume-logo` não vaza nem colide com outro `.lume-logo` em outro arquivo.

## Reatividade: as "runes" (Svelte 5)

Em Svelte você **não** manipula o DOM na mão. Você declara variáveis reativas; quando elas
mudam, a tela se re-desenha sozinha. Essas variáveis usam funções com `$`, as *runes*:

- **`$state(...)`** — variável reativa. Mudou → a UI atualiza. Como uma célula de planilha.
- **`$derived(...)`** — valor calculado a partir de outros (recalcula sozinho).
- **`$props()`** — as entradas que um componente recebe (como atributos de HTML).
- **`$effect(() => ...)`** — roda um efeito colateral quando as dependências mudam.

Em `+page.svelte` o estado inteiro da tela é declarado assim:

```ts
let expanded = $state(!isTauri);              // cápsula recolhida ou aberta?
let sessions = $state<AgentSession[]>([]);     // a lista de sessões
let view = $state<View>("sessions");           // aba atual: sessions/board/history/settings
let selectedId = $state<string | null>(null);  // qual sessão está expandida
```

O ponto-chave: quando o Rust manda dados novos e o código faz `sessions = [...]`, **não há
nenhum código que "redesenha a lista"** — o Svelte percebe que `sessions` mudou e
re-renderiza o `{#each}` que a exibe. Você só cuida dos *dados*; a tela é uma função deles.

O mascote é um bom exemplo de `$derived`: seu status visual é reduzido de todas as sessões
(`permission_required` > `failed` > `running` > `waiting_for_input` > `completed` > `idle`).
Mude qualquer sessão e o rosto muda de cor sem nenhuma linha de "atualizar mascote".

## Sintaxe do template

Cinco construções cobrem quase tudo. Todos os exemplos abaixo são do `+page.svelte`.

**Inserir um valor** — `{ }`:
```svelte
<span class="agent-count">{activeCount}</span>
```

**Condicional** — `{#if}`. É o que troca entre a bolinha recolhida e o painel aberto:
```svelte
{#if !expanded}
  <button class="lume-orb ...">...</button>   <!-- cápsula recolhida -->
{:else}
  <section class="panel">...</section>         <!-- painel aberto -->
{/if}
```

**Loop** — `{#each}`. Desenha a lista de sessões:
```svelte
{#each sessions as session (session.id)}
  <article class="session-row">
    <strong>{session.agentLabel}</strong>
    <span class="project-name">{session.project}</span>
  </article>
{/each}
```
O `(session.id)` no fim é a **chave** — ajuda o Svelte a saber qual item é qual quando a
lista muda, para animar corretamente.

**Eventos** — props que começam com `on`:
```svelte
<button onclick={toggleExpanded}>...</button>
<button onclick={() => handlePermission(session, action)}>...</button>
```

**Classes dinâmicas** — `class:nome={condição}` e interpolação:
```svelte
<button class="lume-orb status-{shellStatus}" class:dragging>
```
`class:dragging` adiciona a classe só quando `dragging` for verdadeira; `status-{shellStatus}`
vira `status-running`, `status-failed`, etc. — é assim que a cor muda por CSS.

**Usar um componente** — como uma tag HTML, passando props:
```svelte
<LumeMascot status={shellStatus} awake={mascotAwake || dragging} size={30} />
```
Esses `status`, `awake` e `size` são exatamente os `$props()` declarados em
`LumeMascot.svelte`.

Juntando tudo, o bloco de permissão (o coração do produto) fica assim:
```svelte
{#if session.pendingPermission}
  <div class="permission-block risk-{session.pendingPermission.risk}">
    <strong>{shown(session.pendingPermission.summary)}</strong>
    <div class="permission-actions">
      {#each session.permissionProfile.availableActions as action}
        <button onclick={() => handlePermission(session, action)}>
          {actionLabel(action)}
        </button>
      {/each}
    </div>
  </div>
{/if}
```
Repare que a regra do produto (só existem os botões que a sessão declara em
`availableActions`) vira UI aqui: o `{#each}` só cria os botões que o backend autorizou. A
interface é uma projeção fiel dos dados, não um `if` para cada ação. Veja como o backend
decide isso em [Arquitetura → Modelo de permissão](ARCHITECTURE.md#modelo-de-permissão).

## Como a UI conversa com o Rust

Nenhum `.svelte` chama o Rust diretamente. Tudo passa por `src/lib/lume.ts`, que embrulha
`invoke` (a ponte do Tauri). Cada função lá corresponde a um comando `#[tauri::command]`
definido em `src-tauri/src/lib.rs`:

```ts
// lume.ts
export async function decidePermission(sessionId, permissionId, action) {
  await invoke("resolve_permission", { sessionId, permissionId, action });
}
```

O caminho inverso (Rust → UI) usa **eventos**. No `onMount` (quando a tela monta), o
`+page.svelte` assina:
```ts
listen("lume://sessions-changed", () => refreshSessions());
```
Então `refreshSessions` chama `loadSessions()` → `sessions = [...]` (o `$state` muda) → o
`{#each}` re-renderiza. É esse laço — **Rust emite evento → recarrega → `$state` muda →
tela re-renderiza** — que faz a cápsula reagir em tempo real. Há também um *poll* de 5 s
como reserva.

## Por que `+page.svelte` é grande

Não é uma tela só: são **quatro abas** no mesmo arquivo, controladas pelo `view` (`$state`):
```svelte
{#if view === "sessions"} ... {/if}   <!-- lista de sessões -->
{#if view === "board"}    ... {/if}   <!-- whiteboard -->
{#if view === "history"}  ... {/if}   <!-- histórico -->
{#if view === "settings"} ... {/if}   <!-- ajustes -->
```
Mais o `<style>` de tudo isso. São efetivamente quatro telas e seus estilos num arquivo.
Além das abas, o arquivo concentra a paleta global de comandos, perfis por projeto, layouts
salvos do Whiteboard, notas de resultados e a instalação dos detectores externos. Os dados e
as operações nativas continuam tipados em `domain.ts` e encapsulados em `lume.ts`.

## Quero mudar X, edito onde

| Você quer mudar…                                   | Vá em…                                        |
| -------------------------------------------------- | --------------------------------------------- |
| Qualquer coisa da cápsula (layout, textos, abas)   | `src/routes/+page.svelte`                     |
| O mascote (rosto, cores, animação)                 | `src/lib/LumeMascot.svelte`                    |
| A janela flutuante do Whiteboard                   | `src/lib/TerminalWindow.svelte`               |
| Ícones dos agentes/navegadores                     | `src/lib/BrandIcon.svelte`                     |
| Textos em pt/en                                    | `src/lib/i18n.ts`                              |
| Adicionar/alterar uma chamada ao Rust              | `src/lib/lume.ts` (+ o comando em `lib.rs`)    |
| Um tipo de dado novo                               | `src/lib/domain.ts` (+ `src-tauri/src/domain.rs`) |
| Perfis, layouts, notas ou paleta de comandos       | `src/routes/+page.svelte` + `src/lib/domain.ts` |
| Catálogo e validação de detectores externos        | `src-tauri/src/agent_plugins.rs`               |
| Estilos globais (fonte, fundo)                     | `src/app.css`                                  |

## Como rodar e ver suas mudanças

Dois modos, e o segundo é ideal para aprender:

1. **`npm run tauri dev`** — abre o app de verdade (com o Rust). Edite um `.svelte`, salve,
   e a janela atualiza na hora (hot reload).
2. **`npm run dev`** e abra `http://localhost:1420` **no navegador**. Aqui a checagem
   `isTauri` é falsa, então o app usa os **dados falsos de `demo.ts`**. Você desenvolve a
   interface inteira, com o DevTools do navegador, sem precisar de nenhum agente rodando —
   é o melhor ambiente para experimentar layout e estilo.
