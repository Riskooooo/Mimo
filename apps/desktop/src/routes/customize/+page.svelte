<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { applyTheme, DEFAULT_ACCENT, PRESETS, watchTheme } from "$lib/theme";
  import Character, { type Mood } from "$lib/Character.svelte";

  // Mimo's look (settings `accent_color`, `tinted_glass`): every window
  // follows it at once (see src/lib/theme.ts).
  type Lang = "fr" | "en";
  type Settings = { language: Lang; accent_color: string; tinted_glass: boolean; character_enabled: boolean };

  const TEXT = {
    fr: {
      title: "Personnaliser",
      subtitle: "Choisis la couleur de Mimo : la pilule, les boutons, les fenêtres.",
      color: "Couleur",
      custom: "Personnalisée",
      customHint: "Choisir une autre couleur",
      character: "Personnage",
      characterHint: "Un petit visage qui réagit, à la place du point. Survole l'aperçu : il te suit des yeux.",
      glass: "Teinter le verre",
      glassHint: "Le fond des fenêtres de Mimo prend une touche de la couleur.",
      preview: "Aperçu",
      previewText: "Prêt",
      reset: "Couleur d'origine",
      close: "Fermer",
    },
    en: {
      title: "Customize",
      subtitle: "Pick Mimo's color: the pill, the buttons, the windows.",
      color: "Color",
      custom: "Custom",
      customHint: "Pick another color",
      character: "Character",
      characterHint: "A little face that reacts, instead of the dot. Hover the preview: its eyes follow you.",
      glass: "Tint the glass",
      glassHint: "The background of Mimo's windows takes a touch of the color.",
      preview: "Preview",
      previewText: "Ready",
      reset: "Original color",
      close: "Close",
    },
  };

  let lang = $state<Lang>("fr");
  let accent = $state(DEFAULT_ACCENT);
  let tinted = $state(false);
  let character = $state(true);
  // The preview plays the character's reactions in turn (and rests,
  // eyes following the pointer, while hovered).
  const DEMO: Mood[] = ["idle", "listening", "thinking", "success", "idle", "error", "suggestion", "sleep"];
  let demoStep = $state(0);
  let previewHovered = $state(false);
  const demoMood = $derived(previewHovered ? "idle" : DEMO[demoStep % DEMO.length]);
  const t = $derived(TEXT[lang] ?? TEXT.fr);
  const isPreset = $derived(PRESETS.some((p) => p.color === accent));

  function take(settings: Settings) {
    lang = settings.language;
    accent = settings.accent_color;
    tinted = settings.tinted_glass;
    character = settings.character_enabled;
  }

  async function save(color: string, glass: boolean, face = character) {
    accent = color.toLowerCase();
    tinted = glass;
    character = face;
    take(
      await invoke<Settings>("set_appearance", { accentColor: accent, tintedGlass: tinted, characterEnabled: character }),
    );
  }

  // Dragging in the color picker previews here only; letting go saves it
  // (and restyles every window).
  function previewCustom(event: Event) {
    accent = (event.currentTarget as HTMLInputElement).value;
    applyTheme({ accent_color: accent, tinted_glass: tinted });
  }

  function saveCustom(event: Event) {
    void save((event.currentTarget as HTMLInputElement).value, tinted);
  }

  function close() {
    void invoke("close_customize_window");
  }

  function load() {
    void invoke<Settings>("get_settings").then(take);
  }

  onMount(watchTheme);

  onMount(() => {
    const demo = setInterval(() => (demoStep += 1), 1900);
    return () => clearInterval(demo);
  });

  onMount(() => {
    load();
    const unlistenFocus = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) load();
    });
    const unlistenSettings = listen<Settings>("mimo://settings-changed", (event) => take(event.payload));
    return () => {
      void unlistenFocus.then((fn) => fn());
      void unlistenSettings.then((fn) => fn());
    };
  });
</script>

<svelte:window onkeydown={(event) => event.key === "Escape" && close()} />

<main class="window">
  <section class="sheet" in:fly={{ y: 14, duration: 420, easing: cubicOut, opacity: 0 }}>
    <header class="header" data-tauri-drag-region>
      <div data-tauri-drag-region>
        <h1 data-tauri-drag-region>{t.title}</h1>
        <p class="subtitle" data-tauri-drag-region>{t.subtitle}</p>
      </div>
      <button class="close" type="button" aria-label={t.close} onclick={close}>
        <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
          <line x1="2" y1="2" x2="10" y2="10" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
          <line x1="10" y1="2" x2="2" y2="10" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
        </svg>
      </button>
    </header>

    <div class="scroll">
      <p class="section-title">{t.preview}</p>
      <div class="preview" role="presentation" onpointerenter={() => (previewHovered = true)} onpointerleave={() => (previewHovered = false)}>
        <div class="mini-pill">
          {#if character}
            <Character mood={demoMood} size={24} />
          {:else}
            <span class="mini-dot"></span>
          {/if}
          <span class="mini-text">Mimo · {t.previewText}</span>
          <span class="mini-switch"><span class="mini-knob"></span></span>
        </div>
        <span class="mini-button">OK</span>
      </div>

      <p class="section-title">{t.color}</p>
      <div class="card swatches">
        {#each PRESETS as preset (preset.color)}
          <button
            class="swatch"
            class:selected={accent === preset.color}
            type="button"
            style={`--c: ${preset.color}`}
            title={preset[lang]}
            aria-label={preset[lang]}
            aria-pressed={accent === preset.color}
            onclick={() => void save(preset.color, tinted)}
          ></button>
        {/each}
        <label class="swatch custom" class:selected={!isPreset} title={t.customHint} style={`--c: ${accent}`}>
          <input type="color" value={accent} oninput={previewCustom} onchange={saveCustom} aria-label={t.custom} />
          <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <line x1="6" y1="2.5" x2="6" y2="9.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
            <line x1="2.5" y1="6" x2="9.5" y2="6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
          </svg>
        </label>
      </div>

      <div class="card">
        <div class="row">
          <div class="row-text">
            <span class="row-title">{t.character}</span>
            <span class="row-detail">{t.characterHint}</span>
          </div>
          <button
            class="switch"
            class:on={character}
            type="button"
            role="switch"
            aria-checked={character}
            aria-label={t.character}
            onclick={() => void save(accent, tinted, !character)}
          >
            <span class="switch-knob"></span>
          </button>
        </div>
        <div class="row">
          <div class="row-text">
            <span class="row-title">{t.glass}</span>
            <span class="row-detail">{t.glassHint}</span>
          </div>
          <button
            class="switch"
            class:on={tinted}
            type="button"
            role="switch"
            aria-checked={tinted}
            aria-label={t.glass}
            onclick={() => void save(accent, !tinted)}
          >
            <span class="switch-knob"></span>
          </button>
        </div>
      </div>

      <button class="reset" type="button" disabled={accent === DEFAULT_ACCENT && !tinted} onclick={() => void save(DEFAULT_ACCENT, false, character)}>
        {t.reset}
      </button>
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

  /* Same solid sheet as the panel and commands windows. */
  .sheet {
    display: flex;
    flex-direction: column;
    height: 100%;
    border-radius: 30px;
    overflow: hidden;
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.08), rgba(255, 255, 255, 0) 30%),
      linear-gradient(160deg, rgba(var(--accent-rgb), var(--tint)), rgba(var(--accent-rgb), 0) 80%),
      linear-gradient(165deg, #26262d, #111115);
    box-shadow:
      0 1px 0 rgba(255, 255, 255, 0.12) inset,
      0 0 0 1px rgba(255, 255, 255, 0.08) inset,
      0 8px 24px rgba(0, 0, 0, 0.4);
    transition: background 0.3s ease;
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
    padding: 4px 18px 18px;
    display: flex;
    flex-direction: column;
  }

  /* Scrolls rather than squashing the cards. */
  .scroll > * {
    flex-shrink: 0;
  }

  .section-title {
    margin: 14px 4px 8px;
    font-size: 0.74rem;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: rgba(245, 245, 247, 0.5);
  }

  .card {
    border-radius: 18px;
    background: rgba(255, 255, 255, 0.06);
    box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.05) inset;
    overflow: hidden;
  }

  /* A miniature of the pill in the chosen color. */
  .preview {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 18px 16px;
    border-radius: 18px;
    background: rgba(0, 0, 0, 0.25);
    box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.05) inset;
  }

  .mini-pill {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 9px;
    height: 38px;
    padding: 0 12px 0 15px;
    border-radius: 999px;
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.16), rgba(255, 255, 255, 0.02) 38%),
      linear-gradient(160deg, rgba(var(--accent-rgb), var(--tint)), rgba(var(--accent-rgb), 0) 80%),
      linear-gradient(160deg, rgba(46, 46, 54, 0.9), rgba(10, 10, 14, 0.95));
    box-shadow:
      0 1px 0 rgba(255, 255, 255, 0.1) inset,
      0 0 0 1px rgba(255, 255, 255, 0.07) inset;
  }

  .mini-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 0 8px rgba(var(--accent-rgb), 0.7);
  }

  .mini-text {
    flex: 1;
    font-size: 0.86rem;
    font-weight: 600;
  }

  .mini-switch {
    position: relative;
    width: 30px;
    height: 18px;
    border-radius: 999px;
    background: var(--accent);
  }

  .mini-knob {
    position: absolute;
    top: 2px;
    left: 14px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #fff;
  }

  .mini-button {
    padding: 8px 16px;
    border-radius: 999px;
    background: var(--accent);
    color: var(--on-accent);
    font-size: 0.82rem;
    font-weight: 600;
  }

  .swatches {
    display: grid;
    grid-template-columns: repeat(7, 1fr);
    gap: 12px;
    padding: 16px;
    justify-items: center;
  }

  .swatch {
    position: relative;
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    padding: 0;
    border: none;
    border-radius: 50%;
    background: var(--c);
    color: #fff;
    cursor: pointer;
    transition: transform 0.15s ease;
  }

  .swatch:hover {
    transform: scale(1.08);
  }

  /* The ring around the chosen color. */
  .swatch.selected {
    box-shadow:
      0 0 0 2.5px #1c1c21,
      0 0 0 4.5px #f5f5f7;
  }

  .swatch.custom {
    background:
      conic-gradient(from 0deg, #ff453a, #ffd60a, #32d74b, #64d2ff, #5e5ce6, #bf5af2, #ff453a);
  }

  .swatch.custom.selected {
    background: var(--c);
  }

  .swatch.custom svg {
    filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.5));
  }

  /* The native picker opens on click, but stays out of sight. */
  .swatch.custom input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    opacity: 0;
    cursor: pointer;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 0;
    padding: 13px 16px;
  }

  .card + .card {
    margin-top: 12px;
  }

  .row + .row {
    box-shadow: 0 -1px 0 rgba(255, 255, 255, 0.06);
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
    color: rgba(245, 245, 247, 0.55);
  }

  .switch {
    position: relative;
    width: 36px;
    height: 21px;
    flex-shrink: 0;
    border: none;
    border-radius: 999px;
    padding: 0;
    background: rgba(255, 255, 255, 0.18);
    cursor: pointer;
    transition: background 0.2s ease;
  }

  .switch.on {
    background: var(--accent);
  }

  .switch-knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 17px;
    height: 17px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
    transition: transform 0.2s cubic-bezier(0.16, 1, 0.3, 1);
  }

  .switch.on .switch-knob {
    transform: translateX(15px);
  }

  .reset {
    align-self: center;
    margin-top: 18px;
    border: none;
    border-radius: 10px;
    padding: 8px 14px;
    background: rgba(255, 255, 255, 0.08);
    color: #f5f5f7;
    font: inherit;
    font-size: 0.82rem;
    font-weight: 600;
    cursor: pointer;
  }

  .reset:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.14);
  }

  .reset:disabled {
    opacity: 0.35;
    cursor: default;
  }
</style>
