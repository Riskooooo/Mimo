<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  let visible = $state(false);

  onMount(() => {
    const unlisten = listen("mimo://tray-menu-open", () => {
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
    };
  });

  function handleSettings() {
    void invoke("open_settings_from_tray");
  }

  function handleClose() {
    void invoke("quit_app");
  }
</script>

<main class="menu" class:visible>
  <button class="menu-item" type="button" onclick={handleSettings}>
    <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
      <line x1="1" y1="3" x2="11" y2="3" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
      <circle cx="4" cy="3" r="1.3" fill="currentColor" />
      <line x1="1" y1="9" x2="11" y2="9" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
      <circle cx="8" cy="9" r="1.3" fill="currentColor" />
    </svg>
    <span>Settings</span>
  </button>
  <button class="menu-item menu-item-close" type="button" onclick={handleClose}>
    <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
      <line x1="2" y1="2" x2="10" y2="10" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
      <line x1="10" y1="2" x2="2" y2="10" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
    </svg>
    <span>Close Mimo</span>
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
    height: 100%;
    padding: 6px;
    box-sizing: border-box;
    border-radius: 16px;
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.16), rgba(255, 255, 255, 0.02) 38%),
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

  .menu-item-close:hover {
    background: #ff453a;
    color: #fff;
  }
</style>
