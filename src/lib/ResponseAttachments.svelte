<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { save as saveDialog } from "@tauri-apps/plugin-dialog";
  import { openPath } from "@tauri-apps/plugin-opener";
  import type { PromptAttachment } from "$lib/domain";
  import type { Language } from "$lib/i18n";
  import { extractResponseFiles, type ResponseFileReference } from "$lib/chatAttachments";
  import { exportLocalFile, readLocalImageDataUrl, setTerminalFileDialogActive } from "$lib/lume";

  let {
    text = "",
    attachments = [],
    workingDirectory,
    language = "en",
    onError = () => undefined,
  } = $props<{
    text?: string;
    attachments?: PromptAttachment[];
    workingDirectory?: string;
    language?: Language;
    onError?: (message: string) => void;
  }>();

  const resources = $derived(
    extractResponseFiles(text, attachments, workingDirectory),
  );
  let previews = $state<Record<string, string>>({});
  let attemptedPreviews = $state<Record<string, boolean>>({});
  let savingPath = $state<string | null>(null);
  const windowLabel = getCurrentWindow().label;

  $effect(() => {
    for (const resource of resources) {
      if (!resource.isImage || attemptedPreviews[resource.path]) continue;
      attemptedPreviews = { ...attemptedPreviews, [resource.path]: true };
      void readLocalImageDataUrl(resource.path)
        .then((source) => {
          previews = { ...previews, [resource.path]: source };
        })
        .catch(() => undefined);
    }
  });

  function tr(english: string, portuguese: string) {
    return language === "pt-BR" ? portuguese : english;
  }

  async function openResource(resource: ResponseFileReference) {
    try {
      await openPath(resource.path);
    } catch (error) {
      onError(String(error).replace(/^Error:\s*/, ""));
    }
  }

  async function downloadResource(resource: ResponseFileReference) {
    if (savingPath) return;
    savingPath = resource.path;
    let dialogLowered = false;
    try {
      try {
        await setTerminalFileDialogActive(windowLabel, true);
        dialogLowered = true;
      } catch {
        // Native save dialogs still work on window managers without layer control.
      }
      const destination = await saveDialog({ defaultPath: resource.name });
      if (typeof destination === "string" && destination) {
        await exportLocalFile(resource.path, destination);
      }
    } catch (error) {
      onError(String(error).replace(/^Error:\s*/, ""));
    } finally {
      if (dialogLowered) {
        await setTerminalFileDialogActive(windowLabel, false).catch(() => undefined);
      }
      savingPath = null;
    }
  }
</script>

{#if resources.length}
  <div class="response-attachments" aria-label={tr("Response files", "Arquivos da resposta")}>
    {#each resources as resource (resource.path)}
      <article class:image={resource.isImage}>
        <button
          class="resource-preview"
          type="button"
          title={tr("Open file", "Abrir arquivo")}
          onclick={() => void openResource(resource)}
        >
          {#if resource.isImage && previews[resource.path]}
            <img src={previews[resource.path]} alt={resource.name} />
          {:else}
            <svg viewBox="0 0 24 24" aria-hidden="true">
              {#if resource.isImage}
                <path d="M4 5h16v14H4zM7 15l3-3 3 3 2-2 3 3M8.5 9.2h.01" />
              {:else}
                <path d="M6 3h8l4 4v14H6zM14 3v5h4M9 13h6M9 17h4" />
              {/if}
            </svg>
          {/if}
        </button>
        <button class="resource-name" type="button" onclick={() => void openResource(resource)}>
          <strong title={resource.name}>{resource.name}</strong>
          <small>{resource.mimeType}</small>
        </button>
        <button
          class="resource-download"
          type="button"
          disabled={savingPath === resource.path}
          title={tr("Save a copy", "Salvar uma cópia")}
          aria-label={tr(`Save ${resource.name}`, `Salvar ${resource.name}`)}
          onclick={() => void downloadResource(resource)}
        >
          <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M10 3v9M6.5 9 10 12.5 13.5 9M4 16h12" /></svg>
        </button>
      </article>
    {/each}
  </div>
{/if}

<style>
  .response-attachments { width: min(360px, 100%); margin-top: 7px; display: grid; gap: 5px; }
  article { min-width: 0; min-height: 42px; padding: 5px; display: grid; grid-template-columns: 34px minmax(0, 1fr) 28px; align-items: center; gap: 7px; overflow: hidden; border: 1px solid rgba(75, 105, 91, 0.13); border-radius: 8px; background: rgba(53, 130, 91, 0.045); }
  article.image:has(img) { grid-template-columns: 74px minmax(0, 1fr) 28px; }
  button { border: 0; color: inherit; background: transparent; cursor: pointer; }
  .resource-preview { width: 34px; height: 32px; padding: 0; display: grid; place-items: center; overflow: hidden; border-radius: 6px; color: #668174; background: rgba(57, 126, 91, 0.075); }
  article.image:has(img) .resource-preview { width: 74px; height: 52px; }
  .resource-preview img { width: 100%; height: 100%; display: block; object-fit: cover; }
  .resource-preview svg { width: 19px; height: 19px; fill: none; stroke: currentColor; stroke-width: 1.35; stroke-linecap: round; stroke-linejoin: round; }
  .resource-name { min-width: 0; padding: 2px 0; display: grid; gap: 2px; text-align: left; }
  .resource-name strong { min-width: 0; overflow: hidden; color: #4c6358; font: 700 var(--chat-small-font-size, 9px)/1.25 Inter, sans-serif; text-overflow: ellipsis; white-space: nowrap; }
  .resource-name small { overflow: hidden; color: #84958c; font: 600 var(--chat-tiny-font-size, 7px)/1.2 Inter, sans-serif; text-overflow: ellipsis; white-space: nowrap; }
  .resource-download { width: 28px; height: 28px; padding: 6px; display: grid; place-items: center; border-radius: 7px; color: #608174; }
  .resource-download:hover:not(:disabled) { color: #31865f; background: rgba(49, 134, 95, 0.09); }
  .resource-download:disabled { opacity: 0.45; cursor: wait; }
  .resource-download svg { width: 15px; height: 15px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
  :global(.terminal-window.dark) article { border-color: rgba(202, 222, 212, 0.09); background: rgba(104, 178, 140, 0.045); }
  :global(.terminal-window.dark) .resource-preview { color: #94b5a5; background: rgba(109, 178, 143, 0.08); }
  :global(.terminal-window.dark) .resource-name strong { color: #bfd0c7; }
  :global(.terminal-window.dark) .resource-name small { color: #7f958a; }
  :global(.terminal-window.dark) .resource-download { color: #8eaa9c; }
</style>
