<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";

  // The user's own commands (see crates/core/src/custom.rs).
  type Lang = "fr" | "en";
  type Action =
    | { kind: "reply"; text: string }
    | { kind: "open_url"; url: string }
    | { kind: "launch_app"; name: string; app_id: string }
    | { kind: "request"; text: string };
  type Kind = Action["kind"];
  type Command = { id: number; trigger: string; action: Action };
  type AppChoice = { name: string; app_id: string };

  const TEXT = {
    fr: {
      title: "Mes commandes",
      subtitle: "Quand tu dis ou écris une phrase, Mimo fait ce que tu veux.",
      empty: "Aucune commande",
      emptyHint: "Ex. : quand je dis « ma chaîne », ouvre youtube.com/@moi.",
      when: "Quand je dis…",
      whenPlaceholder: "ex. ma chaîne",
      then: "Mimo…",
      kinds: { reply: "Répond", open_url: "Ouvre un site", launch_app: "Ouvre une app", request: "Fait une demande" },
      replyPlaceholder: "ex. Salut toi !",
      urlPlaceholder: "ex. youtube.com/@moi",
      requestPlaceholder: "ex. mets du lofi sur youtube",
      pickApp: "Choisis une application…",
      add: "Ajouter",
      save: "Enregistrer",
      cancel: "Annuler",
      edit: "Modifier",
      delete: "Supprimer",
      close: "Fermer",
      says: (t: string) => `Répond « ${t} »`,
      opens: (t: string) => `Ouvre ${t}`,
      asks: (t: string) => `Comme si tu disais « ${t} »`,
      voiceNote: "À la voix, seuls les mots connus du modèle vocal sont compris.",
    },
    en: {
      title: "My commands",
      subtitle: "When you say or type a phrase, Mimo does what you want.",
      empty: "No commands yet",
      emptyHint: "E.g. when I say “my channel”, open youtube.com/@me.",
      when: "When I say…",
      whenPlaceholder: "e.g. my channel",
      then: "Mimo…",
      kinds: { reply: "Replies", open_url: "Opens a site", launch_app: "Opens an app", request: "Makes a request" },
      replyPlaceholder: "e.g. Hey you!",
      urlPlaceholder: "e.g. youtube.com/@me",
      requestPlaceholder: "e.g. play lofi on youtube",
      pickApp: "Pick an app…",
      add: "Add",
      save: "Save",
      cancel: "Cancel",
      edit: "Edit",
      delete: "Delete",
      close: "Close",
      says: (t: string) => `Replies “${t}”`,
      opens: (t: string) => `Opens ${t}`,
      asks: (t: string) => `As if you said “${t}”`,
      voiceNote: "By voice, only words the speech model knows are understood.",
    },
  };
  const KINDS: Kind[] = ["reply", "open_url", "launch_app", "request"];

  let lang = $state<Lang>("en");
  const t = $derived(TEXT[lang] ?? TEXT.en);

  let commands = $state<Command[]>([]);
  let apps = $state<AppChoice[]>([]);

  // The form: a new command (id 0) or the one being edited.
  let editingId = $state(0);
  let trigger = $state("");
  let kind = $state<Kind>("open_url");
  let value = $state("");
  let appId = $state("");
  let error = $state("");
  let triggerInput: HTMLInputElement | undefined = $state();

  function describe(action: Action) {
    switch (action.kind) {
      case "reply":
        return t.says(action.text);
      case "open_url":
        return t.opens(action.url.replace(/^https?:\/\//, ""));
      case "launch_app":
        return t.opens(action.name);
      case "request":
        return t.asks(action.text);
    }
  }

  function placeholder(k: Kind) {
    if (k === "reply") return t.replyPlaceholder;
    if (k === "open_url") return t.urlPlaceholder;
    return t.requestPlaceholder;
  }

  function resetForm() {
    editingId = 0;
    trigger = "";
    value = "";
    appId = "";
    error = "";
  }

  function startEdit(command: Command) {
    editingId = command.id;
    trigger = command.trigger;
    kind = command.action.kind;
    error = "";
    const action = command.action;
    value = action.kind === "reply" || action.kind === "request" ? action.text : action.kind === "open_url" ? action.url : "";
    appId = action.kind === "launch_app" ? action.app_id : "";
    triggerInput?.focus();
  }

  function action(): Action {
    switch (kind) {
      case "reply":
        return { kind, text: value };
      case "open_url":
        return { kind, url: value };
      case "request":
        return { kind, text: value };
      case "launch_app": {
        const app = apps.find((a) => a.app_id === appId);
        return { kind, name: app?.name ?? "", app_id: app?.app_id ?? "" };
      }
    }
  }

  async function save() {
    try {
      commands = await invoke<Command[]>("save_custom_command", {
        command: { id: editingId, trigger, action: action() },
      });
      resetForm();
    } catch (err) {
      error = String(err);
    }
  }

  async function remove(id: number) {
    commands = await invoke<Command[]>("delete_custom_command", { id });
    if (editingId === id) resetForm();
  }

  function formKeydown(event: KeyboardEvent) {
    if (event.key === "Enter") {
      event.preventDefault();
      void save();
    }
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key !== "Escape") return;
    if (editingId) resetForm();
    else void invoke("close_commands_window");
  }

  async function load() {
    const settings = await invoke<{ language: Lang }>("get_settings");
    lang = settings.language;
    commands = await invoke<Command[]>("list_custom_commands");
    apps = await invoke<AppChoice[]>("list_installed_apps");
  }

  onMount(() => {
    void load();
    // The window is only hidden between uses: reload when it comes back.
    const unlistenFocus = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) void load();
    });
    const unlistenSettings = listen<{ language: Lang }>("mimo://settings-changed", (event) => {
      lang = event.payload.language;
    });
    return () => {
      void unlistenFocus.then((fn) => fn());
      void unlistenSettings.then((fn) => fn());
    };
  });
</script>

<svelte:window onkeydown={handleKeydown} />

<main class="window">
  <section class="sheet" in:fly={{ y: 14, duration: 420, easing: cubicOut, opacity: 0 }}>
    <header class="header" data-tauri-drag-region>
      <div data-tauri-drag-region>
        <h1 data-tauri-drag-region>{t.title}</h1>
        <p class="subtitle" data-tauri-drag-region>{t.subtitle}</p>
      </div>
      <button class="close" type="button" aria-label={t.close} onclick={() => void invoke("close_commands_window")}>
        <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
          <line x1="2" y1="2" x2="10" y2="10" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
          <line x1="10" y1="2" x2="2" y2="10" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
        </svg>
      </button>
    </header>

    <div class="scroll">
      {#if commands.length === 0}
        <div class="empty">
          <svg class="empty-glyph" viewBox="0 0 24 24" aria-hidden="true"><path d="M5 6.5h14M5 12h9M5 17.5h6" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /><path d="M17 14.5v6M14 17.5h6" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>
          <span class="empty-title">{t.empty}</span>
          <span class="empty-hint">{t.emptyHint}</span>
        </div>
      {:else}
        <div class="card list">
          {#each commands as command (command.id)}
            <div class="row" class:selected={editingId === command.id}>
              <div class="row-text">
                <span class="row-title">« {command.trigger} »</span>
                <span class="row-detail">{describe(command.action)}</span>
              </div>
              <button class="icon edit" type="button" aria-label={t.edit} title={t.edit} onclick={() => startEdit(command)}>
                <svg viewBox="0 0 12 12" width="11" height="11" aria-hidden="true"><path d="M7.6 2.4l2 2L4.4 9.6 2 10l.4-2.4z" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" /></svg>
              </button>
              <button class="icon delete" type="button" aria-label={t.delete} title={t.delete} onclick={() => remove(command.id)}>
                <svg viewBox="0 0 12 12" width="11" height="11" aria-hidden="true"><line x1="2.5" y1="6" x2="9.5" y2="6" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" /></svg>
              </button>
            </div>
          {/each}
        </div>
      {/if}
    </div>

    <div class="editor">
      <label class="field-label" for="trigger">{t.when}</label>
      <input
        id="trigger"
        type="text"
        spellcheck="false"
        autocomplete="off"
        placeholder={t.whenPlaceholder}
        bind:this={triggerInput}
        bind:value={trigger}
        oninput={() => (error = "")}
        onkeydown={formKeydown}
      />
      <span class="field-label">{t.then}</span>
      <div class="chips" role="radiogroup">
        {#each KINDS as k (k)}
          <button
            class="chip"
            class:on={kind === k}
            type="button"
            role="radio"
            aria-checked={kind === k}
            onclick={() => {
              kind = k;
              error = "";
            }}>{t.kinds[k]}</button
          >
        {/each}
      </div>
      {#if kind === "launch_app"}
        <select bind:value={appId} onchange={() => (error = "")}>
          <option value="">{t.pickApp}</option>
          {#each apps as app (app.app_id)}
            <option value={app.app_id}>{app.name}</option>
          {/each}
        </select>
      {:else}
        <input
          type="text"
          spellcheck="false"
          autocomplete="off"
          placeholder={placeholder(kind)}
          bind:value
          oninput={() => (error = "")}
          onkeydown={formKeydown}
        />
      {/if}
      {#if error}
        <p class="error">{error}</p>
      {:else}
        <p class="note">{t.voiceNote}</p>
      {/if}
      <div class="actions">
        {#if editingId}
          <button class="button" type="button" onclick={resetForm}>{t.cancel}</button>
        {/if}
        <button class="button primary" type="button" onclick={save}>{editingId ? t.save : t.add}</button>
      </div>
    </div>
  </section>
</main>

<style>
  :global(html),
  :global(body) {
    margin: 0;
    background: transparent;
    overflow: hidden;
    font-family:
      -apple-system,
      "SF Pro Display",
      "Segoe UI Variable Display",
      "Segoe UI",
      Inter,
      sans-serif;
    color: #f5f5f7;
    -webkit-font-smoothing: antialiased;
    user-select: none;
    -webkit-user-select: none;
  }

  .window {
    width: 100vw;
    height: 100vh;
    box-sizing: border-box;
    padding: 14px;
  }

  /* Same solid sheet as the panel window. */
  .sheet {
    display: flex;
    flex-direction: column;
    height: 100%;
    border-radius: 30px;
    overflow: hidden;
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.08), rgba(255, 255, 255, 0) 30%),
      linear-gradient(165deg, #26262d, #111115);
    box-shadow:
      0 1px 0 rgba(255, 255, 255, 0.12) inset,
      0 0 0 1px rgba(255, 255, 255, 0.08) inset,
      0 8px 24px rgba(0, 0, 0, 0.4);
  }

  .header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
    padding: 22px 22px 10px;
  }

  h1 {
    margin: 0;
    font-size: 1.65rem;
    font-weight: 700;
    letter-spacing: -0.02em;
  }

  .subtitle {
    margin: 4px 0 0;
    font-size: 0.86rem;
    color: rgba(245, 245, 247, 0.62);
  }

  .close {
    flex-shrink: 0;
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border: none;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.12);
    color: rgba(245, 245, 247, 0.85);
    cursor: pointer;
  }

  .close:hover {
    background: rgba(255, 255, 255, 0.2);
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 8px 18px 12px;
  }

  .card {
    border-radius: 18px;
    background: rgba(255, 255, 255, 0.06);
    box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.05) inset;
    overflow: hidden;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 11px 14px;
  }

  .row + .row {
    box-shadow: 0 -1px 0 rgba(255, 255, 255, 0.06);
  }

  .row.selected {
    background: rgba(10, 132, 255, 0.12);
  }

  .row-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }

  .row-title {
    font-size: 0.9rem;
    font-weight: 600;
  }

  .row-detail {
    font-size: 0.78rem;
    color: rgba(245, 245, 247, 0.6);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .icon {
    flex-shrink: 0;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    padding: 0;
    border: none;
    border-radius: 50%;
    cursor: pointer;
  }

  .icon.edit {
    background: rgba(255, 255, 255, 0.07);
    color: rgba(245, 245, 247, 0.6);
  }

  .icon.edit:hover {
    background: rgba(10, 132, 255, 0.2);
    color: #0a84ff;
  }

  .icon.delete {
    background: rgba(255, 69, 58, 0.16);
    color: #ff6961;
  }

  .icon.delete:hover {
    background: rgba(255, 69, 58, 0.3);
  }

  .editor {
    display: flex;
    flex-direction: column;
    gap: 7px;
    margin: 0 18px 18px;
    padding: 14px;
    border-radius: 18px;
    background: rgba(255, 255, 255, 0.07);
    box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.05) inset;
  }

  .field-label {
    font-size: 0.74rem;
    font-weight: 600;
    letter-spacing: 0.02em;
    text-transform: uppercase;
    color: rgba(245, 245, 247, 0.5);
  }

  .editor input,
  .editor select {
    min-width: 0;
    border: none;
    outline: none;
    border-radius: 10px;
    padding: 8px 11px;
    background: rgba(255, 255, 255, 0.08);
    color: #f5f5f7;
    font: inherit;
    font-size: 0.88rem;
    color-scheme: dark;
    user-select: text;
    -webkit-user-select: text;
  }

  .editor input:focus,
  .editor select:focus {
    box-shadow: 0 0 0 1.5px #0a84ff inset;
  }

  .editor input::placeholder {
    color: rgba(245, 245, 247, 0.38);
  }

  .editor select option {
    background: #1c1c21;
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .chip {
    border: none;
    border-radius: 999px;
    padding: 6px 11px;
    background: rgba(255, 255, 255, 0.08);
    color: rgba(245, 245, 247, 0.8);
    font: inherit;
    font-size: 0.8rem;
    font-weight: 600;
    cursor: pointer;
  }

  .chip.on {
    background: #0a84ff;
    color: #fff;
  }

  .error,
  .note {
    margin: 0;
    font-size: 0.74rem;
  }

  .error {
    color: #ff6961;
  }

  .note {
    color: rgba(245, 245, 247, 0.4);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }

  .button {
    border: none;
    border-radius: 10px;
    padding: 7px 14px;
    background: rgba(255, 255, 255, 0.08);
    color: #f5f5f7;
    font: inherit;
    font-size: 0.84rem;
    font-weight: 600;
    cursor: pointer;
  }

  .button.primary {
    background: #0a84ff;
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 40px 24px;
    text-align: center;
  }

  .empty-glyph {
    width: 44px;
    height: 44px;
    color: rgba(245, 245, 247, 0.35);
    margin-bottom: 6px;
  }

  .empty-title {
    font-size: 1rem;
    font-weight: 600;
  }

  .empty-hint {
    font-size: 0.8rem;
    color: rgba(245, 245, 247, 0.5);
    max-width: 300px;
  }
</style>
