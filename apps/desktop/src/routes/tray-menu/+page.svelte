<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
  import { tick } from "svelte";
  import { onMount } from "svelte";
  import { watchTheme } from "$lib/theme";

  type Language = "en" | "fr";
  type MenuSettings = { language: Language; ai_enabled: boolean };
  // See src-tauri/src/ai (same as the pill's settings row).
  type AiStatus =
    | { state: "off"; installed: boolean; size: number }
    | { state: "downloading"; done: number; total: number }
    | { state: "ready" }
    | { state: "error"; message: string };
  const TEXT = {
    en: {
      ai: "Local AI",
      aiReady: "Ready",
      aiOff: "Off",
      aiError: "Download failed",
      settings: "Settings",
      customize: "Customize",
      close: "Close Mimo",
      problem: "A problem?",
    },
    fr: {
      ai: "IA locale",
      aiReady: "Prête",
      aiOff: "Désactivée",
      aiError: "Échec du téléchargement",
      settings: "Réglages",
      customize: "Personnaliser",
      close: "Fermer Mimo",
      problem: "Un problème ?",
    },
  };

  let visible = $state(false);
  let language = $state<Language>("en");
  let aiEnabled = $state(false);
  let aiStatus = $state<AiStatus>({ state: "off", installed: false, size: 0 });
  const t = $derived(TEXT[language] ?? TEXT.en);
  const aiDetail = $derived.by(() => {
    if (aiStatus.state === "downloading") {
      return `${aiStatus.total > 0 ? Math.floor((aiStatus.done * 100) / aiStatus.total) : 0} %`;
    }
    if (aiStatus.state === "ready") return t.aiReady;
    if (aiStatus.state === "error") return t.aiError;
    return t.aiOff;
  });

  let menu = $state<HTMLElement>();

  // The window takes exactly the items' height (whatever the display
  // scaling or language), so nothing gets cut off. The shell places it
  // from its size when the tray icon is right-clicked.
  async function fitWindow() {
    await tick();
    if (!menu) return;
    const height = Math.ceil(menu.scrollHeight);
    if (Math.abs(height - window.innerHeight) > 1) {
      await getCurrentWindow().setSize(new LogicalSize(window.innerWidth, height));
    }
  }

  function apply(settings: MenuSettings) {
    language = settings.language;
    aiEnabled = settings.ai_enabled;
    void fitWindow();
  }

  function loadSettings() {
    void invoke<MenuSettings>("get_settings").then(apply);
    void invoke<AiStatus>("get_ai_status").then((status) => (aiStatus = status));
  }

  onMount(watchTheme);

  onMount(() => {
    loadSettings();
    const unlistenSettings = listen<MenuSettings>("mimo://settings-changed", (event) => apply(event.payload));
    const unlistenAi = listen<AiStatus>("mimo://ai-status", (event) => {
      aiStatus = event.payload;
    });
    const unlisten = listen("mimo://tray-menu-open", () => {
      loadSettings();
      // Reset then re-trigger so re-opening after a quick close still
      // replays the entrance animation instead of just staying visible.
      visible = false;
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          visible = true;
        });
      });
    });

    return () => {
      void unlisten.then((fn) => fn());
      void unlistenSettings.then((fn) => fn());
      void unlistenAi.then((fn) => fn());
    };
  });

  // The menu stays open: the switch shows the new state (and the download
  // progress, the first time).
  async function toggleAi() {
    apply(await invoke<MenuSettings>("set_ai_enabled", { enabled: !aiEnabled }));
    aiStatus = await invoke<AiStatus>("get_ai_status");
  }

  function handleCustomize() {
    void invoke("open_customize_window");
  }

  function handleSettings() {
    void invoke("open_settings_from_tray");
  }

  // Bug reports go to the GitHub issues (the browser taking focus closes
  // the menu, like any click outside it).
  function handleProblem() {
    void openUrl("https://github.com/Riskooooo/Mimo/issues");
  }

  function handleClose() {
    void invoke("quit_app");
  }
</script>

<main class="menu" class:visible bind:this={menu}>
  <button class="menu-item ai-item" type="button" role="switch" aria-checked={aiEnabled} onclick={toggleAi}>
    <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
      <path d="M6 1.2l1.15 3.1 3.1 1.15-3.1 1.15L6 9.7 4.85 6.6 1.75 5.45l3.1-1.15z" fill="currentColor" />
      <circle cx="10" cy="10" r="1" fill="currentColor" />
    </svg>
    <span class="ai-text">
      <span>{t.ai}</span>
      <span class="ai-detail" class:failed={aiStatus.state === "error"}>{aiDetail}</span>
    </span>
    <span class="switch" class:on={aiEnabled}><span class="switch-knob"></span></span>
  </button>
  <div class="separator" role="separator"></div>
  <button class="menu-item" type="button" onclick={handleSettings}>
    <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
      <line x1="1" y1="3" x2="11" y2="3" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
      <circle cx="4" cy="3" r="1.3" fill="currentColor" />
      <line x1="1" y1="9" x2="11" y2="9" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
      <circle cx="8" cy="9" r="1.3" fill="currentColor" />
    </svg>
    <span>{t.settings}</span>
  </button>
  <button class="menu-item" type="button" onclick={handleCustomize}>
    <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
      <circle cx="6" cy="6" r="4.8" fill="none" stroke="currentColor" stroke-width="1.2" />
      <path d="M6 1.2a4.8 4.8 0 0 1 0 9.6z" fill="currentColor" />
    </svg>
    <span>{t.customize}</span>
  </button>
  <button class="menu-item menu-item-close" type="button" onclick={handleClose}>
    <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
      <line x1="2" y1="2" x2="10" y2="10" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
      <line x1="10" y1="2" x2="2" y2="10" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
    </svg>
    <span>{t.close}</span>
  </button>
  <div class="separator" role="separator"></div>
  <button class="menu-item" type="button" onclick={handleProblem}>
    <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
      <circle cx="6" cy="6" r="4.8" fill="none" stroke="currentColor" stroke-width="1.2" />
      <path d="M4.6 4.7a1.45 1.45 0 1 1 2.1 1.3c-.45.24-.7.55-.7 1v.25" fill="none" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" />
      <circle cx="6" cy="8.75" r=".6" fill="currentColor" />
    </svg>
    <span>{t.problem}</span>
  </button>
</main>

<style>
  :global(html),
  :global(body) {
    margin: 0;
    background: transparent;
    overflow: hidden;
    user-select: none;
    -webkit-user-select: none;
  }

  .menu {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    /* Its own height (the window is fitted to it, see fitWindow). */
    height: auto;
    padding: 6px;
    box-sizing: border-box;
    border-radius: 16px;
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.16), rgba(255, 255, 255, 0.02) 38%),
      linear-gradient(160deg, rgba(var(--accent-rgb), var(--tint)), rgba(var(--accent-rgb), 0) 80%),
      linear-gradient(160deg, rgba(46, 46, 54, 0.85), rgba(10, 10, 14, 0.92));
    backdrop-filter: blur(22px) saturate(165%);
    -webkit-backdrop-filter: blur(22px) saturate(165%);
    box-shadow:
      0 1px 0 rgba(255, 255, 255, 0.1) inset,
      0 0 0 1px rgba(255, 255, 255, 0.07) inset,
      0 10px 26px rgba(0, 0, 0, 0.4);
    transform: scale(0.92) translateY(6px);
    opacity: 0;
    transition:
      transform 0.28s cubic-bezier(0.16, 1, 0.3, 1),
      opacity 0.22s ease;
  }

  .menu.visible {
    transform: scale(1) translateY(0);
    opacity: 1;
  }

  .menu-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 9px 10px;
    border: none;
    border-radius: 10px;
    background: transparent;
    color: #f5f5f7;
    font-family:
      -apple-system,
      "SF Pro Display",
      "Segoe UI",
      Inter,
      sans-serif;
    font-size: 0.85rem;
    font-weight: 500;
    cursor: pointer;
    text-align: left;
    transition: background 0.12s ease;
  }

  .menu-item:hover {
    background: rgba(255, 255, 255, 0.1);
  }

  .separator {
    height: 1px;
    margin: 3px 8px;
    flex-shrink: 0;
    background: rgba(255, 255, 255, 0.1);
  }

  .ai-text {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    line-height: 1.2;
  }

  .ai-detail {
    font-size: 0.7rem;
    font-weight: 400;
    color: rgba(245, 245, 247, 0.5);
  }

  .ai-detail.failed {
    color: #ff9f97;
  }

  /* Same switch as the pill's settings, a size smaller. */
  .switch {
    position: relative;
    width: 30px;
    height: 18px;
    flex-shrink: 0;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.18);
    transition: background 0.2s ease;
  }

  .switch.on {
    background: var(--accent);
  }

  .switch-knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
    transition: transform 0.2s cubic-bezier(0.16, 1, 0.3, 1);
  }

  .switch.on .switch-knob {
    transform: translateX(12px);
  }

  .menu-item-close:hover {
    background: #ff453a;
    color: #fff;
  }
</style>
