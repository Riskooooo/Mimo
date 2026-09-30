<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { fade } from "svelte/transition";
  import { cubicOut } from "svelte/easing";

  type EngineStatus = { running: boolean };
  type AppSettings = { launch_at_startup: boolean };

  // The pill grows in two hops, like the real Dynamic Island: a small
  // "compact" size while loading, then a wider "full" size once ready.
  // Kept in sync with the `.pill` transition duration below.
  const PILL_TRANSITION_MS = 550;
  const COMPACT_REVEAL_DELAY_MS = 260;

  // Shown while the engine's initial state (later: data loaded into memory)
  // is being fetched, so the pill never has to pop straight to its final
  // content the instant that becomes slow. Kept comfortably longer than
  // COMPACT_REVEAL_DELAY_MS so the loading spinner is actually visible for a
  // beat before the pill grows again and swaps to the ready content.
  const MIN_LOADING_MS = 1200;

  // Once ready, let "Mimo" sit centered for a beat before shifting left to
  // make room for the action buttons — mirrors how the Dynamic Island
  // settles before revealing controls.
  const CONTROLS_REVEAL_DELAY_MS = 600;

  // Long enough to cover the status-recenter + button-fade-out beat that
  // happens before the pill itself starts shrinking on minimize.
  const CLOSE_PREAMBLE_MS = 320;

  // Kept in sync with `.settings-panel`/`.pill.settings-open`'s transition
  // duration below.
  const SETTINGS_TRANSITION_MS = 400;

  const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

  type Stage = "collapsed" | "compact" | "full";
  type Phase = "loading" | "ready" | "closing";

  let stage = $state<Stage>("collapsed");
  let phase = $state<Phase>("loading");
  let engineRunning = $state(false);
  let contentVisible = $state(false);
  let showControls = $state(false);
  let settingsOpen = $state(false);
  let settings = $state<AppSettings>({ launch_at_startup: false });

  async function playInitialReveal() {
    const start = performance.now();

    // Double rAF: let the collapsed state paint first, otherwise the browser
    // may coalesce the class change into the same frame and skip the
    // transition entirely.
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        stage = "compact";
      });
    });

    setTimeout(() => {
      contentVisible = true;
    }, COMPACT_REVEAL_DELAY_MS);

    const status = await invoke<EngineStatus>("engine_status");
    const elapsed = performance.now() - start;
    if (elapsed < MIN_LOADING_MS) {
      await new Promise((resolve) => setTimeout(resolve, MIN_LOADING_MS - elapsed));
    }

    engineRunning = status.running;
    phase = "ready";
    stage = "full";

    setTimeout(() => {
      showControls = true;
    }, CONTROLS_REVEAL_DELAY_MS);
  }

  // Re-opening from the tray: the engine's state is already known, so skip
  // the loading spinner and the centered pause — just grow straight back to
  // the already-settled "ready" layout.
  function playQuickReveal() {
    stage = "collapsed";

    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        stage = "full";
      });
    });
  }

  // Grows the window first (invisibly, since the drawer starts collapsed),
  // then triggers the CSS reveal — the same pre-size-then-animate trick used
  // for the pill's own reveal, so the drawer's growth is a smooth CSS
  // transition rather than snapping to a native window resize.
  async function openSettings() {
    if (settingsOpen) return;
    await invoke("set_settings_panel_open", { open: true });
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        settingsOpen = true;
      });
    });
  }

  async function closeSettings() {
    if (!settingsOpen) return;
    settingsOpen = false;
    await sleep(SETTINGS_TRANSITION_MS);
    await invoke("set_settings_panel_open", { open: false });
  }

  function toggleSettings() {
    if (settingsOpen) {
      void closeSettings();
    } else {
      void openSettings();
    }
  }

  async function toggleLaunchAtStartup() {
    settings = await invoke<AppSettings>("set_launch_at_startup", {
      enabled: !settings.launch_at_startup,
    });
  }

  async function handleEraseMemory() {
    settings = await invoke<AppSettings>("erase_memory");
  }

  onMount(() => {
    void playInitialReveal();
    void invoke<AppSettings>("get_settings").then((loaded) => {
      settings = loaded;
    });

    const unlistenReveal = listen("mimo://reveal", () => {
      playQuickReveal();
    });

    const unlistenOpenSettings = listen("mimo://open-settings", () => {
      void (async () => {
        await sleep(PILL_TRANSITION_MS + 100);
        await openSettings();
      })();
    });

    return () => {
      void unlistenReveal.then((fn) => fn());
      void unlistenOpenSettings.then((fn) => fn());
    };
  });

  async function handleMinimize() {
    if (phase !== "ready") return;

    await closeSettings();

    // Let the buttons fade out and the status recenter first, then retract
    // the pill back to a dot before actually hiding the window.
    showControls = false;
    await sleep(CLOSE_PREAMBLE_MS);

    phase = "closing";
    stage = "collapsed";
    await sleep(PILL_TRANSITION_MS + 60);

    await invoke("hide_window");

    // Reset back to the settled "ready" layout (window is hidden, so this is
    // invisible) so the next tray reveal just has to grow the pill again.
    phase = "ready";
    showControls = true;
  }

  function handleClose() {
    void invoke("quit_app");
  }
</script>

<main class="stage">
  <div
    class="pill"
    class:compact={stage === "compact"}
    class:full={stage === "full"}
    class:settings-open={settingsOpen}
    data-tauri-drag-region
  >
    <div class="sheen"></div>

    {#if phase === "loading"}
      <div class="content" class:visible={contentVisible}>
        <div class="row" out:fade={{ duration: 320, easing: cubicOut }} in:fade={{ duration: 220, easing: cubicOut }}>
          <span class="spinner" aria-hidden="true">
            {#each { length: 8 } as _, i (i)}
              <span class="tick" style={`--i: ${i}`}></span>
            {/each}
          </span>
          <span class="label">Loading…</span>
        </div>
      </div>
    {:else if phase === "ready"}
      <div
        class="bar"
        in:fade={{ duration: 380, delay: 120, easing: cubicOut }}
        out:fade={{ duration: 220, easing: cubicOut }}
      >
        <div class="status" class:shifted={showControls}>
          <span class="dot running"></span>
          <span class="label">Mimo</span>
        </div>

        <div class="actions" class:visible={showControls}>
          <button class="action" type="button" aria-label="Settings" onclick={toggleSettings}>
            <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
              <line x1="1" y1="3" x2="11" y2="3" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
              <circle cx="4" cy="3" r="1.3" fill="currentColor" />
              <line x1="1" y1="9" x2="11" y2="9" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
              <circle cx="8" cy="9" r="1.3" fill="currentColor" />
            </svg>
          </button>
          <button class="action" type="button" aria-label="Minimize Mimo" onclick={handleMinimize}>
            <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
              <line x1="2" y1="6" x2="10" y2="6" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
            </svg>
          </button>
          <button class="action action-close" type="button" aria-label="Close Mimo" onclick={handleClose}>
            <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
              <line x1="2" y1="2" x2="10" y2="10" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
              <line x1="10" y1="2" x2="2" y2="10" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
            </svg>
          </button>
        </div>
      </div>

      <div class="settings-panel" class:open={settingsOpen}>
        <div class="settings-row">
          <span class="settings-label">Launch at login</span>
          <button
            class="switch"
            class:on={settings.launch_at_startup}
            type="button"
            role="switch"
            aria-checked={settings.launch_at_startup}
            aria-label="Launch at login"
            onclick={toggleLaunchAtStartup}
          >
            <span class="switch-knob"></span>
          </button>
        </div>
        <button class="settings-action" type="button" onclick={handleEraseMemory}>Erase memory</button>
      </div>
    {/if}
  </div>
</main>

<style>
  :global(html),
  :global(body) {
    margin: 0;
    background: transparent;
    overflow: hidden;
    /* Without this, a mousedown+drag on the pill can start a native text
       selection (or drag-image) gesture alongside Tauri's window drag,
       which shows up as a stray selection highlight while dragging. */
    user-select: none;
    -webkit-user-select: none;
  }

  .stage {
    width: 100vw;
    height: 100vh;
    display: flex;
    /* Top-anchored on purpose, not centered: the settings drawer grows the
       window's height natively (instant, unanimated), and if the pill were
       vertically centered, that resize alone would snap it to a new
       position before its own CSS transition even starts. A fixed top
       offset keeps the pill's top edge glued in place regardless of window
       height, so only its bottom edge ever visibly moves. */
    align-items: flex-start;
    justify-content: center;
  }

  .pill {
    position: relative;
    display: flex;
    flex-direction: column;
    margin-top: 20px;
    width: 32px;
    height: 32px;
    border-radius: 999px;
    overflow: hidden;
    /* CSS-only "glass" look: real native blur-behind (Windows Acrylic) was
       tried and rolled back — it requires DWM frame/backdrop plumbing that
       turned out too fragile for a fully borderless window (see project
       notes). This gradient plus backdrop-filter is what carries the whole
       effect, so it needs to stay reasonably opaque. */
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.16), rgba(255, 255, 255, 0.02) 38%),
      linear-gradient(160deg, rgba(46, 46, 54, 0.78), rgba(10, 10, 14, 0.88));
    backdrop-filter: blur(22px) saturate(165%);
    -webkit-backdrop-filter: blur(22px) saturate(165%);
    /* Keep the outer shadow's reach (offset + blur) comfortably inside the
       20px bottom margin the window gives the pill (see WINDOW_MARGIN_Y in
       src-tauri/src/lib.rs) — otherwise it gets hard-clipped by the window
       edge instead of fading out. */
    box-shadow:
      0 1px 0 rgba(255, 255, 255, 0.1) inset,
      0 0 0 1px rgba(255, 255, 255, 0.07) inset,
      0 4px 14px rgba(0, 0, 0, 0.32);
    /* border-radius deliberately NOT in this list: the browser auto-clamps
       a large radius down to half of the CURRENT height, so animating it
       at the same time height grows makes the radius temporarily balloon
       past its own target (tracking height/2 upward before settling back
       down), which is large enough to round straight over the buttons near
       the top corners. Snapping it instead is invisible in practice, since
       at the instant it changes the clamp already renders both values
       identically. */
    transition:
      width 0.55s cubic-bezier(0.16, 1, 0.3, 1),
      height 0.55s cubic-bezier(0.16, 1, 0.3, 1);
  }

  /* Small, content-sized pill used for the loading spinner — only as wide as
     it needs to be, not the final "ready" width. */
  .pill.compact {
    width: 150px;
    height: 44px;
  }

  /* Final size once ready: grows outward from the compact pill. A fixed
     height (not window-relative) is what lets .settings-open below animate
     smoothly via CSS instead of snapping with the native window resize. */
  .pill.full {
    width: calc(100% - 56px);
    height: 44px;
    transition:
      width 0.55s cubic-bezier(0.16, 1, 0.3, 1),
      height 0.4s cubic-bezier(0.16, 1, 0.3, 1);
  }

  /* Grows downward to reveal .settings-panel. The window is resized to fit
     this *before* the class is added (see openSettings in the script), so
     the extra room already exists and this is a pure CSS-driven grow —
     matches SETTINGS_PANEL_HEIGHT (140) + the base 44px in src-tauri/src/lib.rs. */
  .pill.full.settings-open {
    height: 184px;
    border-radius: 28px;
  }

  .sheen {
    position: absolute;
    inset: 0;
    background: linear-gradient(
      115deg,
      transparent 20%,
      rgba(255, 255, 255, 0.18) 45%,
      transparent 65%
    );
    transform: translateX(-120%);
    opacity: 0;
    pointer-events: none;
  }

  .pill.full .sheen {
    animation: sheen 1.1s cubic-bezier(0.16, 1, 0.3, 1) 0.05s both;
  }

  @keyframes sheen {
    0% {
      transform: translateX(-120%);
      opacity: 0;
    }
    35% {
      opacity: 0.9;
    }
    100% {
      transform: translateX(120%);
      opacity: 0;
    }
  }

  .content {
    margin: auto;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 18px;
    white-space: nowrap;
    opacity: 0;
    transform: translateY(2px);
    transition:
      opacity 0.4s ease,
      transform 0.4s ease;
    /* Decorative only — let clicks fall through to .pill's drag region. */
    pointer-events: none;
  }

  .content.visible {
    opacity: 1;
    transform: translateY(0);
  }

  .row {
    position: absolute;
    display: flex;
    align-items: center;
    gap: 0.55rem;
  }

  /* Ready state: status starts perfectly centered (plain CSS, no measured
     widths involved — nothing to race or jump) and shifts to the left edge
     once the controls are revealed. Fixed height matches PILL_HEIGHT so it
     never grows/shrinks when the settings drawer opens below it. */
  .bar {
    position: relative;
    flex: 0 0 44px;
    width: 100%;
    /* Decorative wrapper — let clicks fall through to .pill's drag region;
       .actions re-enables pointer events for the buttons specifically. */
    pointer-events: none;
  }

  .status {
    position: absolute;
    top: 50%;
    left: 50%;
    display: flex;
    align-items: center;
    gap: 0.55rem;
    transform: translate(-50%, -50%);
    transition:
      left 0.4s cubic-bezier(0.16, 1, 0.3, 1),
      transform 0.4s cubic-bezier(0.16, 1, 0.3, 1);
  }

  .status.shifted {
    left: 18px;
    transform: translate(0, -50%);
  }

  .actions {
    position: absolute;
    top: 50%;
    right: 18px;
    display: flex;
    align-items: center;
    gap: 0.4rem;
    opacity: 0;
    transform: translate(10px, -50%);
    transition:
      opacity 0.3s ease,
      transform 0.4s cubic-bezier(0.16, 1, 0.3, 1);
    pointer-events: none;
  }

  .actions.visible {
    opacity: 1;
    transform: translate(0, -50%);
    pointer-events: auto;
  }

  .action {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border: none;
    border-radius: 50%;
    padding: 0;
    background: rgba(255, 255, 255, 0.1);
    color: #f5f5f7;
    cursor: pointer;
    transition:
      background 0.15s ease,
      transform 0.1s ease;
  }

  .action:hover {
    background: rgba(255, 255, 255, 0.2);
  }

  .action:active {
    transform: scale(0.9);
  }

  .action-close:hover {
    background: #ff453a;
    color: #fff;
  }

  /* Settings drawer: collapsed to zero height, revealed below the bar once
     the pill itself has grown to make room (see .pill.full.settings-open). */
  .settings-panel {
    flex: 0 0 0;
    width: 100%;
    box-sizing: border-box;
    overflow: hidden;
    opacity: 0;
    padding: 0 18px;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 10px;
    pointer-events: none;
    transition:
      flex-basis 0.4s cubic-bezier(0.16, 1, 0.3, 1),
      opacity 0.3s ease;
  }

  .settings-panel.open {
    flex-basis: 140px;
    opacity: 1;
    pointer-events: auto;
  }

  .settings-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }

  .settings-label {
    font-family:
      -apple-system,
      "SF Pro Display",
      "Segoe UI",
      Inter,
      sans-serif;
    font-size: 0.82rem;
    font-weight: 500;
    color: #f5f5f7;
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
    background: #32d74b;
  }

  .switch-knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 17px;
    height: 17px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.4);
    transition: transform 0.2s cubic-bezier(0.16, 1, 0.3, 1);
  }

  .switch.on .switch-knob {
    transform: translateX(15px);
  }

  .settings-action {
    align-self: flex-start;
    border: none;
    border-radius: 8px;
    padding: 6px 10px;
    background: rgba(255, 69, 58, 0.16);
    color: #ff6961;
    font-family:
      -apple-system,
      "SF Pro Display",
      "Segoe UI",
      Inter,
      sans-serif;
    font-size: 0.78rem;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.15s ease;
  }

  .settings-action:hover {
    background: rgba(255, 69, 58, 0.28);
  }

  .dot {
    width: 8px;
    height: 8px;
    flex-shrink: 0;
    border-radius: 50%;
    background: #6e6e73;
    box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.08);
    transition:
      background 0.2s ease,
      box-shadow 0.2s ease;
  }

  .dot.running {
    background: #32d74b;
    box-shadow: 0 0 8px rgba(50, 215, 75, 0.7);
  }

  .spinner {
    position: relative;
    width: 16px;
    height: 16px;
    flex-shrink: 0;
  }

  .tick {
    position: absolute;
    top: 0;
    left: 50%;
    width: 2px;
    height: 42%;
    margin-left: -1px;
    border-radius: 1px;
    background: #f5f5f7;
    transform-origin: 50% 8px;
    transform: rotate(calc(var(--i) * 45deg));
    opacity: 0.25;
    animation: tick-fade 1s linear infinite;
    animation-delay: calc(var(--i) * -125ms);
  }

  @keyframes tick-fade {
    0% {
      opacity: 1;
    }
    100% {
      opacity: 0.25;
    }
  }

  .label {
    font-family:
      -apple-system,
      "SF Pro Display",
      "Segoe UI",
      Inter,
      sans-serif;
    font-size: 0.92rem;
    font-weight: 590;
    letter-spacing: 0.01em;
    color: #f5f5f7;
    user-select: none;
  }
</style>
