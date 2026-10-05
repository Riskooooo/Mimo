<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { getVersion } from "@tauri-apps/api/app";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { onMount } from "svelte";
  import { watchTheme } from "$lib/theme";
  import Character, { type Mood } from "$lib/Character.svelte";
  import { fade } from "svelte/transition";
  import { cubicOut } from "svelte/easing";

  type EngineStatus = { running: boolean };
  type AppSettings = {
    launch_at_startup: boolean;
    summon_shortcut: string;
    voice_wake_enabled: boolean;
    language: Language;
    sounds_enabled: boolean;
    activity_enabled: boolean;
    suggestions_enabled: boolean;
    ai_enabled: boolean;
    accent_color: string;
    tinted_glass: boolean;
    character_enabled: boolean;
  };
  // The local AI's download/readiness (see src-tauri/src/ai).
  type AiStatus =
    | { state: "off"; installed: boolean; size: number }
    | { state: "downloading"; done: number; total: number }
    | { state: "ready" }
    | { state: "error"; message: string };
  type Language = "en" | "fr";

  // Every word the pill and its settings show, per the Language setting
  // (which also sets Mimo's replies and the language it listens in).
  const TEXT = {
    en: {
      loading: "Loading…",
      listening: "Listening…",
      placeholder: "Ask Mimo… e.g. “open youtube”",
      stop: "Stop",
      translation: "Translation",
      copy: "Copy",
      copied: "Copied",
      settings: "Settings",
      minimize: "Minimize Mimo",
      close: "Close Mimo",
      launchAtLogin: "Launch at login",
      shortcut: "Open Mimo shortcut",
      changeShortcut: "Change the shortcut that opens Mimo",
      pressKeys: "Press keys…",
      bareKeyNote: "This key can't be typed anywhere else while Mimo runs.",
      voiceWake: "Listen for “Hey Mimo”",
      language: "Language",
      sounds: "Sounds",
      activity: "Analyze my activity",
      suggestions: "Suggestions",
      ai: "Local AI",
      customize: "Customize",
      aiDelete: "Delete",
      aiDeleteTitle: "Delete the AI's files from this PC",
      aiError: "The AI download failed. Turn it off and on to retry.",
      gb: "GB",
      suggestion: "Suggestion",
      open: "Open",
      closeApp: "Close it",
      emptyBin: "Empty it",
      myCommands: "My commands",
      manage: "Manage",
      ok: "OK",
      later: "Later",
      never: "Don't suggest again",
      eraseMemory: "Erase memory",
      help: "Help",
      helpQuestion: "Question about Mimo",
      helpEmergency: "I need emergency help",
      emergency: "In an emergency, call 911 (US) or 112 (Europe) right away.",
      version: "Version",
    },
    fr: {
      loading: "Chargement…",
      listening: "J'écoute…",
      placeholder: "Demande à Mimo… ex. « ouvre youtube »",
      stop: "Arrêter",
      translation: "Traduction",
      copy: "Copier",
      copied: "Copié",
      settings: "Réglages",
      minimize: "Réduire Mimo",
      close: "Fermer Mimo",
      launchAtLogin: "Lancer au démarrage",
      shortcut: "Raccourci pour ouvrir Mimo",
      changeShortcut: "Changer le raccourci qui ouvre Mimo",
      pressKeys: "Appuie sur une touche…",
      bareKeyNote: "Cette touche ne pourra plus être tapée ailleurs tant que Mimo tourne.",
      voiceWake: "Écouter « Hey Mimo »",
      language: "Langue",
      sounds: "Sons",
      activity: "Analyser mon activité",
      suggestions: "Suggestions",
      ai: "IA locale",
      customize: "Personnaliser",
      aiDelete: "Supprimer",
      aiDeleteTitle: "Supprimer les fichiers de l'IA de ce PC",
      aiError: "Le téléchargement de l'IA a échoué. Désactive puis réactive pour réessayer.",
      gb: "Go",
      suggestion: "Suggestion",
      open: "Ouvrir",
      closeApp: "Le fermer",
      emptyBin: "La vider",
      myCommands: "Mes commandes",
      manage: "Gérer",
      ok: "OK",
      later: "Plus tard",
      never: "Ne plus proposer",
      eraseMemory: "Effacer la mémoire",
      help: "Aide",
      helpQuestion: "Question sur Mimo",
      helpEmergency: "J'ai besoin des secours",
      emergency: "En cas d'urgence, appelle tout de suite le 112 (ou le 15 SAMU, 17 police, 18 pompiers).",
      version: "Version",
    },
  };
  type Translation = { original: string; translated: string; from: string; to: string };
  // What the card under the bar shows: a translation, an AI answer, a
  // summary of copied text… (`label`: in the bar; `chip`: above the text).
  type Card = { label: string; chip: string; text: string; original: string | null };
  type AskResponse = {
    ok: boolean;
    reply: string;
    answer: boolean;
    translation: Translation | null;
    awaiting_copy: boolean;
    help: boolean;
    card: { title: string; text: string; original: string | null } | null;
  };
  type VoiceResult = { text: string | null; error: string | null };
  // "reminder": brought up by a due reminder/alarm (mode comes with the
  // separate mimo://ringing event).
  type SummonSource = "shortcut" | "voice" | "reminder";
  type SoundName = "wake" | "success" | "error" | "reminder" | "alarm";
  type Ringing = { kind: "reminder" | "alarm"; message: string | null; time: string };
  // Something Mimo offers on its own (see src-tauri/src/suggestions.rs).
  type Suggestion = {
    keys: string[];
    message: string;
    action:
      | { kind: "launch"; apps: { name: string; app_id: string }[] }
      | { kind: "close"; label: string; process: string }
      | { kind: "empty_recycle_bin" }
      | null;
  };
  type SuggestionChoice = "accept" | "later" | "never" | "dismiss";

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

  // Mimo lives in the background: once shown without being asked for
  // anything, the pill tucks itself away after this much inactivity.
  const IDLE_HIDE_MS = 1500;

  // Typed prompt left untouched (e.g. Windows refused to give it focus, so
  // no blur will ever come to dismiss it).
  const PROMPT_IDLE_HIDE_MS = 10000;

  // An alarm rings (sound repeated) until dismissed, giving up after a few
  // minutes; a reminder chimes once and stays up a while.
  const ALARM_REPEAT_MS = 2400;
  const ALARM_GIVE_UP_MS = 3 * 60 * 1000;
  const REMINDER_LINGER_MS = 30 * 1000;

  // How long Mimo's reply stays readable before the pill tucks away —
  // longer for an actual answer (time, weather) than for "Opening…".
  const REPLY_LINGER_MS = 1800;
  const ANSWER_LINGER_MS = 6000;

  // The translation card under the bar (window grows by this much — see
  // set_pill_drawer), and how long it stays when not hovered.
  const TRANSLATION_DRAWER_HEIGHT = 170;
  const TRANSLATION_LINGER_MS = 20000;

  // The suggestion card (same idea), left up this long unless hovered.
  const SUGGESTION_DRAWER_HEIGHT = 118;
  const SUGGESTION_LINGER_MS = 25000;

  const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

  // The "?" button and "I need help" lead to the project's page.
  const GITHUB_URL = "https://github.com/Riskooooo/Mimo";

  type Stage = "collapsed" | "compact" | "full";
  type Phase = "loading" | "ready" | "closing";
  // What the ready pill is doing: plain status bar, typed prompt, waiting
  // for a spoken request, running a request, or showing the reply.
  // "copywait": waiting for the user to copy text to translate;
  // "translation": the translation card is open.
  // "suggestion": Mimo brought itself up to offer something.
  type Mode =
    | "idle"
    | "input"
    | "listening"
    | "thinking"
    | "reply"
    | "ringing"
    | "copywait"
    | "translation"
    | "suggestion"
    | "help";

  let stage = $state<Stage>("collapsed");
  let phase = $state<Phase>("loading");
  let engineRunning = $state(false);
  let contentVisible = $state(false);
  let showControls = $state(false);
  let settingsOpen = $state(false);
  let settings = $state<AppSettings>({
    launch_at_startup: false,
    summon_shortcut: "F9",
    voice_wake_enabled: true,
    language: "en",
    sounds_enabled: true,
    activity_enabled: true,
    suggestions_enabled: true,
    ai_enabled: false,
    accent_color: "#0a84ff",
    tinted_glass: false,
    character_enabled: true,
  });
  let aiStatus = $state<AiStatus>({ state: "off", installed: false, size: 0 });
  const aiPercent = $derived(
    aiStatus.state === "downloading" && aiStatus.total > 0 ? Math.floor((aiStatus.done * 100) / aiStatus.total) : 0,
  );

  const t = $derived(TEXT[settings.language] ?? TEXT.en);

  // The character's face for what the pill is doing.
  const mood = $derived.by((): Mood => {
    switch (mode) {
      case "listening":
      case "copywait":
        return "listening";
      case "thinking":
        return "thinking";
      case "reply":
        return replyOk ? "success" : "error";
      case "suggestion":
      case "help":
        return "suggestion";
      case "ringing":
        return "alarm";
      case "translation":
        return "success";
      default: {
        // Drowsy in the middle of the night.
        const hour = new Date().getHours();
        return hour >= 1 && hour < 5 ? "sleep" : "idle";
      }
    }
  });

  let mode = $state<Mode>("idle");
  let request = $state("");
  let reply = $state("");
  let replyOk = $state(true);
  let promptInput = $state<HTMLInputElement>();
  let capturingShortcut = $state(false);
  let shortcutError = $state("");
  // Shown right after binding a key without Ctrl/Alt/Win (allowed, but the
  // key is then grabbed system-wide).
  let shortcutNote = $state("");
  // Accelerators name physical keys (QWERTY positions: "Q" is the key
  // labelled A on AZERTY), so they're shown through the user's layout.
  let layoutMap = $state<Map<string, string> | null>(null);
  try {
    (navigator as any).keyboard?.getLayoutMap?.().then((map: Map<string, string>) => (layoutMap = map), () => {});
  } catch {
    // Keyboard Map API unavailable: shortcuts show their QWERTY names.
  }
  let voiceError = $state("");
  let ringing = $state<Ringing | null>(null);
  let ringTimers: ReturnType<typeof setTimeout>[] = [];
  let card = $state<Card | null>(null);
  let drawerOpen = $state(false);
  let copied = $state(false);
  let translationTimer: ReturnType<typeof setTimeout> | undefined;
  let suggestion = $state<Suggestion | null>(null);
  // "J'ai besoin d'aide": the question asked, then the emergency numbers
  // if that's what it is (shown in the suggestion drawer).
  let helpText = $state("");
  let helpEmergency = $state(false);
  let appVersion = $state("");
  let suggestionOpen = $state(false);
  let suggestionTimer: ReturnType<typeof setTimeout> | undefined;

  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  let pointerInside = false;
  let minimizing: Promise<void> | null = null;
  let pendingSummon: SummonSource | null = null;
  let lastSource: SummonSource = "shortcut";

  // Played natively by the shell (which also checks the Sounds setting).
  function sound(name: SoundName) {
    void invoke("play_sound", { sound: name });
  }

  function cancelIdleHide() {
    clearTimeout(idleTimer);
    idleTimer = undefined;
  }

  // Re-armed whenever the pill is left alone: after a reveal, when the
  // pointer leaves it, when settings close, or while the prompt sits idle.
  function armIdleHide() {
    cancelIdleHide();
    if (phase !== "ready" || settingsOpen || pointerInside || capturingShortcut) return;
    if (mode === "idle") {
      idleTimer = setTimeout(() => void handleMinimize(), IDLE_HIDE_MS);
    } else if (mode === "input") {
      idleTimer = setTimeout(() => void handleMinimize(), PROMPT_IDLE_HIDE_MS);
    }
  }

  function handlePointerEnter() {
    pointerInside = true;
    if (mode === "idle") cancelIdleHide();
    // Reading the translation or suggestion: keep it up.
    if (mode === "translation") clearTimeout(translationTimer);
    if (mode === "suggestion" || mode === "help") clearTimeout(suggestionTimer);
  }

  function handlePointerLeave() {
    pointerInside = false;
    if (mode === "idle") armIdleHide();
    if (mode === "translation") armTranslationHide();
    if (mode === "suggestion") armSuggestionHide();
    if (mode === "help") armHelpHide();
  }

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

    if (pendingSummon) {
      const source = pendingSummon;
      pendingSummon = null;
      enterMode(source);
      return;
    }

    setTimeout(() => {
      if (mode !== "idle") return;
      showControls = true;
      armIdleHide();
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

  // The shell has just shown + focused the window (whether it was hidden
  // or already up) and asks for a mode based on how Mimo was summoned.
  async function summon(source: SummonSource) {
    cancelIdleHide();
    if (source === "reminder") {
      // Just make sure the pill is up; startRinging sets the content.
      if (minimizing) await minimizing;
      await closeSettings();
      if (stage !== "full") playQuickReveal();
      return;
    }
    if (phase === "loading") {
      pendingSummon = source;
      return;
    }
    if (minimizing) await minimizing;
    if (mode === "copywait") void invoke("cancel_translation");
    if (mode === "suggestion") void invoke("answer_suggestion", { choice: "dismiss" });
    await closeSettings();
    await closeDrawer();
    card = null;
    suggestion = null;
    if (stage !== "full") playQuickReveal();
    enterMode(source);
  }

  function enterMode(source: SummonSource) {
    lastSource = source;
    showControls = false;
    reply = "";
    if (source === "shortcut") {
      // A fresh prompt every time (it may still hold the last request if
      // summoned again while its reply was showing).
      request = "";
      mode = "input";
      focusPrompt();
      armIdleHide();
    } else {
      mode = "listening";
    }
  }

  function focusPrompt() {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => promptInput?.focus());
    });
  }

  async function submitRequest(text: string) {
    const trimmed = text.trim();
    if (!trimmed) {
      void handleMinimize();
      return;
    }

    cancelIdleHide();
    mode = "thinking";
    reply = trimmed;
    let ok = false;
    let answer = false;
    try {
      const response = await invoke<AskResponse>("ask_mimo", {
        request: trimmed,
        spoken: lastSource === "voice",
      });
      reply = response.reply;
      ok = response.ok;
      answer = response.answer;
      if (response.translation) {
        await showTranslation(response.translation);
        return;
      }
      if (response.card) {
        const { title, text, original } = response.card;
        await showCard({ label: title, chip: t.ai, text, original });
        return;
      }
      if (response.awaiting_copy) {
        await translateNextCopy();
        return;
      }
      if (response.help) {
        await showHelp(response.reply);
        return;
      }
    } catch (err) {
      reply = String(err);
    }
    replyOk = ok;
    mode = "reply";
    sound(ok ? "success" : "error");

    await sleep(answer ? ANSWER_LINGER_MS : REPLY_LINGER_MS);
    if (mode !== "reply") return;

    // A typed request that didn't work goes back to the prompt with the
    // text kept, so it can be fixed instead of retyped.
    if (!ok && lastSource === "shortcut") {
      request = trimmed;
      mode = "input";
      focusPrompt();
      armIdleHide();
    } else {
      await handleMinimize();
    }
  }

  // "traduis" with no text: show the hint until something gets copied.
  async function translateNextCopy() {
    mode = "copywait";
    try {
      const result = await invoke<Translation>("translate_clipboard");
      if (mode !== "copywait") return;
      await showTranslation(result);
    } catch (err) {
      if (mode !== "copywait") return;
      reply = String(err);
      replyOk = false;
      mode = "reply";
      sound("error");
      await sleep(REPLY_LINGER_MS);
      if (mode === "reply") await handleMinimize();
    }
  }

  function showTranslation(next: Translation) {
    return showCard({
      label: t.translation,
      chip: `${next.from.toUpperCase()} → ${next.to.toUpperCase()}`,
      text: next.translated,
      original: next.original,
    });
  }

  async function showCard(next: Card) {
    card = next;
    copied = false;
    replyOk = true;
    mode = "translation";
    sound("success");
    // Same pre-size-then-animate trick as the settings drawer.
    await invoke("set_pill_drawer", { height: TRANSLATION_DRAWER_HEIGHT });
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        drawerOpen = true;
      });
    });
    armTranslationHide();
  }

  function armTranslationHide() {
    clearTimeout(translationTimer);
    if (mode !== "translation" || pointerInside) return;
    translationTimer = setTimeout(() => void dismissTranslation(), TRANSLATION_LINGER_MS);
  }

  async function closeDrawer() {
    clearTimeout(translationTimer);
    clearTimeout(suggestionTimer);
    if (!drawerOpen && !suggestionOpen) return;
    drawerOpen = false;
    suggestionOpen = false;
    await sleep(SETTINGS_TRANSITION_MS);
    await invoke("set_pill_drawer", { height: 0 });
  }

  // The shell has shown the pill (without focus) with something to offer.
  async function showSuggestion(next: Suggestion) {
    if (minimizing) await minimizing;
    await closeSettings();
    cancelIdleHide();
    suggestion = next;
    showControls = false;
    mode = "suggestion";
    if (stage !== "full") playQuickReveal();
    sound("reminder");
    await invoke("set_pill_drawer", { height: SUGGESTION_DRAWER_HEIGHT });
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        suggestionOpen = true;
      });
    });
    armSuggestionHide();
  }

  function armSuggestionHide() {
    clearTimeout(suggestionTimer);
    if (mode !== "suggestion" || pointerInside) return;
    suggestionTimer = setTimeout(() => void answerSuggestion("dismiss"), SUGGESTION_LINGER_MS);
  }

  async function answerSuggestion(choice: SuggestionChoice) {
    if (mode !== "suggestion") return;
    clearTimeout(suggestionTimer);
    const answer = await invoke<string | null>("answer_suggestion", { choice });
    await closeDrawer();
    suggestion = null;
    if (answer) {
      reply = answer;
      replyOk = true;
      mode = "reply";
      sound("success");
      await sleep(REPLY_LINGER_MS);
      if (mode !== "reply") return;
    }
    await handleMinimize();
  }

  async function showHelp(question: string) {
    helpText = question;
    helpEmergency = false;
    replyOk = true;
    mode = "help";
    sound("success");
    await invoke("set_pill_drawer", { height: SUGGESTION_DRAWER_HEIGHT });
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        suggestionOpen = true;
      });
    });
    armHelpHide();
  }

  function armHelpHide() {
    clearTimeout(suggestionTimer);
    if (mode !== "help" || pointerInside) return;
    suggestionTimer = setTimeout(() => void handleMinimize(), SUGGESTION_LINGER_MS);
  }

  async function openHelpPage() {
    await openUrl(GITHUB_URL);
    if (mode === "help") await handleMinimize();
  }

  function acceptLabel(next: Suggestion) {
    if (next.action?.kind === "launch") return t.open;
    if (next.action?.kind === "close") return t.closeApp;
    if (next.action?.kind === "empty_recycle_bin") return t.emptyBin;
    return t.ok;
  }

  async function dismissTranslation() {
    if (mode !== "translation") return;
    await handleMinimize();
  }

  async function copyTranslation() {
    if (!card) return;
    try {
      await invoke("copy_text", { text: card.text });
      copied = true;
    } catch {
      copied = false;
    }
  }

  function handlePromptKeydown(event: KeyboardEvent) {
    if (event.key === "Enter") {
      event.preventDefault();
      void submitRequest(request);
    } else if (event.key === "Escape") {
      event.preventDefault();
      void handleMinimize();
    } else {
      armIdleHide();
    }
  }

  async function handleVoiceResult(result: VoiceResult) {
    if (mode !== "listening") return;
    if (result.text) {
      await submitRequest(result.text);
    } else if (result.error) {
      // Spoken commands unavailable (e.g. online speech off): say why,
      // then fall back to typing so the request isn't lost.
      reply = result.error;
      replyOk = false;
      mode = "reply";
      sound("error");
      await sleep(REPLY_LINGER_MS + 1200);
      if (mode !== "reply") return;
      lastSource = "shortcut";
      mode = "input";
      focusPrompt();
      armIdleHide();
    } else {
      await handleMinimize();
    }
  }

  function startRinging(next: Ringing) {
    // Several due at the same moment: show them together.
    if (mode === "ringing" && ringing) {
      const messages = [ringing.message, next.message].filter(Boolean);
      next = {
        kind: ringing.kind === "alarm" || next.kind === "alarm" ? "alarm" : "reminder",
        time: next.time,
        message: messages.length ? messages.join(" · ") : null,
      };
    }
    stopRinging();
    cancelIdleHide();
    ringing = next;
    showControls = false;
    mode = "ringing";
    if (next.kind === "alarm") {
      sound("alarm");
      const repeat = setInterval(() => sound("alarm"), ALARM_REPEAT_MS);
      ringTimers.push(repeat as unknown as ReturnType<typeof setTimeout>);
      ringTimers.push(setTimeout(() => void dismissRinging(), ALARM_GIVE_UP_MS));
    } else {
      sound("reminder");
      ringTimers.push(setTimeout(() => void dismissRinging(), REMINDER_LINGER_MS));
    }
  }

  function stopRinging() {
    for (const timer of ringTimers) {
      clearTimeout(timer);
      clearInterval(timer);
    }
    ringTimers = [];
  }

  async function dismissRinging() {
    if (mode !== "ringing") return;
    stopRinging();
    ringing = null;
    await handleMinimize();
  }

  // Grows the window first (invisibly, since the drawer starts collapsed),
  // then triggers the CSS reveal — the same pre-size-then-animate trick used
  // for the pill's own reveal, so the drawer's growth is a smooth CSS
  // transition rather than snapping to a native window resize.
  async function openSettings() {
    if (settingsOpen) return;
    cancelIdleHide();
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
    armIdleHide();
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

  async function toggleVoiceWake() {
    voiceError = "";
    settings = await invoke<AppSettings>("set_voice_wake_enabled", {
      enabled: !settings.voice_wake_enabled,
    });
  }

  async function toggleSounds() {
    settings = await invoke<AppSettings>("set_sounds_enabled", {
      enabled: !settings.sounds_enabled,
    });
    // Let the user hear what they just turned on.
    sound("wake");
  }

  async function toggleSuggestions() {
    settings = await invoke<AppSettings>("set_suggestions_enabled", {
      enabled: !settings.suggestions_enabled,
    });
  }

  async function toggleAi() {
    settings = await invoke<AppSettings>("set_ai_enabled", { enabled: !settings.ai_enabled });
    aiStatus = await invoke<AiStatus>("get_ai_status");
  }

  async function deleteAiFiles() {
    aiStatus = await invoke<AiStatus>("delete_ai_files");
  }

  function gigabytes(bytes: number): string {
    const value = (bytes / 1e9).toFixed(1);
    return `${settings.language === "fr" ? value.replace(".", ",") : value} ${t.gb}`;
  }

  async function toggleActivity() {
    settings = await invoke<AppSettings>("set_activity_enabled", {
      enabled: !settings.activity_enabled,
    });
  }

  async function setLanguage(language: Language) {
    if (language === settings.language) return;
    voiceError = "";
    shortcutError = "";
    shortcutNote = "";
    settings = await invoke<AppSettings>("set_language", { language });
  }

  function startShortcutCapture() {
    shortcutError = "";
    shortcutNote = "";
    capturingShortcut = true;
  }

  // KeyboardEvent.code -> the accelerator syntax the global-shortcut plugin
  // parses ("KeyM" -> "M", "Digit1" -> "1"; F-keys, Space, arrows... as-is).
  function acceleratorKey(code: string): string | null {
    if (/^(Control|Shift|Alt|Meta|OS)(Left|Right)?$/.test(code)) return null;
    if (code.startsWith("Key")) return code.slice(3);
    if (code.startsWith("Digit")) return code.slice(5);
    return code;
  }

  function shortcutLabel(shortcut: string): string {
    return shortcut
      .split("+")
      .map((part) => {
        const code = /^[A-Z]$/.test(part) ? `Key${part}` : /^[0-9]$/.test(part) ? `Digit${part}` : part;
        const shown = layoutMap?.get(code)?.trim();
        return shown ? shown.toUpperCase() : part;
      })
      .join("+");
  }

  async function handleShortcutCapture(event: KeyboardEvent) {
    if (mode === "ringing" && event.key === "Escape") {
      event.preventDefault();
      void dismissRinging();
      return;
    }
    if ((mode === "translation" || mode === "copywait") && event.key === "Escape") {
      event.preventDefault();
      void handleMinimize();
      return;
    }
    if (!capturingShortcut) return;
    event.preventDefault();
    event.stopPropagation();

    if (event.key === "Escape") {
      capturingShortcut = false;
      return;
    }
    const key = acceleratorKey(event.code);
    if (!key) return; // only a modifier so far; wait for the real key

    const parts = [
      event.ctrlKey && "Ctrl",
      event.altKey && "Alt",
      event.shiftKey && "Shift",
      event.metaKey && "Super",
      key,
    ].filter(Boolean);

    capturingShortcut = false;
    try {
      settings = await invoke<AppSettings>("set_summon_shortcut", { shortcut: parts.join("+") });
      const grabsTyping = !(event.ctrlKey || event.altKey || event.metaKey) && !/^F([1-9]|1[0-9]|2[0-4])$/.test(key);
      shortcutNote = grabsTyping ? t.bareKeyNote : "";
    } catch (err) {
      shortcutError = String(err);
    }
  }

  async function handleEraseMemory() {
    voiceError = "";
    shortcutError = "";
    shortcutNote = "";
    settings = await invoke<AppSettings>("erase_memory");
  }

  onMount(watchTheme);

  onMount(() => {
    void playInitialReveal();
    void getVersion().then((version) => (appVersion = version));
    void invoke<AppSettings>("get_settings").then((loaded) => {
      settings = loaded;
    });
    void invoke<AiStatus>("get_ai_status").then((status) => (aiStatus = status));
    const unlistenAi = listen<AiStatus>("mimo://ai-status", (event) => {
      aiStatus = event.payload;
    });
    // Changed elsewhere too (the tray menu's AI switch, "Personnaliser").
    const unlistenSettings = listen<AppSettings>("mimo://settings-changed", (event) => {
      settings = event.payload;
    });

    const unlistenReveal = listen("mimo://reveal", () => {
      cancelIdleHide();
      if (mode === "suggestion") {
        void invoke("answer_suggestion", { choice: "dismiss" });
        void closeDrawer();
        suggestion = null;
      }
      stopRinging();
      ringing = null;
      mode = "idle";
      showControls = true;
      playQuickReveal();
      setTimeout(armIdleHide, PILL_TRANSITION_MS);
    });

    const unlistenSummon = listen<SummonSource>("mimo://summon", (event) => {
      void summon(event.payload);
    });

    const unlistenVoiceResult = listen<VoiceResult>("mimo://voice-result", (event) => {
      void handleVoiceResult(event.payload);
    });

    const unlistenRinging = listen<Ringing>("mimo://ringing", (event) => {
      startRinging(event.payload);
    });

    const unlistenSuggestion = listen<Suggestion>("mimo://suggestion", (event) => {
      void showSuggestion(event.payload);
    });

    const unlistenVoiceError = listen<string>("mimo://voice-error", (event) => {
      voiceError = event.payload;
    });

    // Clicking elsewhere dismisses the typed prompt, like any popup.
    const unlistenFocus = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (!focused && mode === "input") void handleMinimize();
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
      void unlistenSummon.then((fn) => fn());
      void unlistenVoiceResult.then((fn) => fn());
      void unlistenVoiceError.then((fn) => fn());
      void unlistenFocus.then((fn) => fn());
      void unlistenRinging.then((fn) => fn());
      void unlistenSuggestion.then((fn) => fn());
      void unlistenAi.then((fn) => fn());
      void unlistenSettings.then((fn) => fn());
      stopRinging();
      cancelIdleHide();
    };
  });

  // Shared by the minimize button, the idle timer and dismissing a prompt;
  // concurrent callers (e.g. Escape + blur) all wait on the same animation.
  function handleMinimize(): Promise<void> {
    if (minimizing) return minimizing;
    if (phase !== "ready") return Promise.resolve();
    minimizing = playMinimize().finally(() => {
      minimizing = null;
    });
    return minimizing;
  }

  async function playMinimize() {
    cancelIdleHide();
    stopRinging();
    capturingShortcut = false;
    if (mode === "copywait") void invoke("cancel_translation");
    await closeSettings();
    await closeDrawer();
    cancelIdleHide();

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
    mode = "idle";
    request = "";
    reply = "";
    card = null;
    suggestion = null;
    showControls = true;
  }

  function handleClose() {
    void invoke("quit_app");
  }
</script>

<svelte:window onkeydowncapture={handleShortcutCapture} />

<main class="stage">
  <div
    class="pill"
    class:compact={stage === "compact"}
    class:full={stage === "full"}
    class:settings-open={settingsOpen}
    class:drawer-open={drawerOpen}
    class:suggestion-open={suggestionOpen}
    data-tauri-drag-region
    role="presentation"
    onpointerenter={handlePointerEnter}
    onpointerleave={handlePointerLeave}
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
          <span class="label">{t.loading}</span>
        </div>
      </div>
    {:else if phase === "ready"}
      <div
        class="bar"
        in:fade={{ duration: 380, delay: 120, easing: cubicOut }}
        out:fade={{ duration: 220, easing: cubicOut }}
      >
        <div class="status" class:shifted={showControls} class:hidden={mode !== "idle"}>
          {#if settings.character_enabled}
            <Character {mood} />
          {:else}
            <span class="dot running"></span>
          {/if}
          <span class="label">Mimo</span>
        </div>

        <div class="actions" class:visible={showControls && mode === "idle"}>
          <button class="action" type="button" aria-label={t.help} title={t.help} onclick={() => void openUrl(GITHUB_URL)}>
            <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
              <path d="M4.1 4.3a1.95 1.95 0 1 1 2.8 1.75c-.6.3-.9.7-.9 1.3v.35" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
              <circle cx="6" cy="9.7" r=".85" fill="currentColor" />
            </svg>
          </button>
          <button class="action" type="button" aria-label={t.settings} onclick={toggleSettings}>
            <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
              <line x1="1" y1="3" x2="11" y2="3" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
              <circle cx="4" cy="3" r="1.3" fill="currentColor" />
              <line x1="1" y1="9" x2="11" y2="9" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
              <circle cx="8" cy="9" r="1.3" fill="currentColor" />
            </svg>
          </button>
          <button class="action" type="button" aria-label={t.minimize} onclick={handleMinimize}>
            <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
              <line x1="2" y1="6" x2="10" y2="6" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
            </svg>
          </button>
          <button class="action action-close" type="button" aria-label={t.close} onclick={handleClose}>
            <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
              <line x1="2" y1="2" x2="10" y2="10" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
              <line x1="10" y1="2" x2="2" y2="10" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
            </svg>
          </button>
        </div>

        {#if mode !== "idle"}
          <div class="prompt" in:fade={{ duration: 220, delay: 80, easing: cubicOut }}>
            {#if settings.character_enabled}
              <Character {mood} />
            {:else}
              <span
                class="dot running"
                class:listening={mode === "listening" || mode === "copywait"}
                class:thinking={mode === "thinking"}
                class:failed={mode === "reply" && !replyOk}
                class:alarm={mode === "ringing"}
                class:suggesting={mode === "suggestion"}
              ></span>
            {/if}
            {#if mode === "input"}
              <input
                class="prompt-input"
                type="text"
                placeholder={t.placeholder}
                spellcheck="false"
                autocomplete="off"
                bind:this={promptInput}
                bind:value={request}
                onkeydown={handlePromptKeydown}
              />
            {:else if mode === "listening"}
              <span class="label prompt-text">{t.listening}</span>
            {:else if mode === "translation"}
              <span class="label prompt-text">{card?.label ?? t.translation}</span>
            {:else if mode === "suggestion"}
              <span class="label prompt-text">{t.suggestion}</span>
            {:else if mode === "help"}
              <span class="label prompt-text">{t.help}</span>
            {:else if mode === "ringing" && ringing}
              <span class="label prompt-text ring-text">
                <span class="ring-time">{ringing.time}</span>
                {#if ringing.message}<span class="ring-message">{ringing.message}</span>{/if}
              </span>
              <button class="stop-button" type="button" onclick={dismissRinging}>{t.stop}</button>
            {:else}
              <span class="label prompt-text" class:muted={mode === "thinking"}>{reply}</span>
            {/if}
          </div>
        {/if}
      </div>

      <div class="translation-panel" class:open={drawerOpen}>
        {#if card}
          <div class="translation-head">
            <span class="lang-chip">{card.chip}</span>
            <button class="copy-button" class:copied type="button" onclick={copyTranslation}>
              {copied ? t.copied : t.copy}
            </button>
          </div>
          <p class="translation-text">{card.text}</p>
          {#if card.original}<p class="translation-original">{card.original}</p>{/if}
        {/if}
      </div>

      <div class="suggestion-panel" class:open={suggestionOpen}>
        {#if mode === "help"}
          <p class="suggestion-text">{helpEmergency ? t.emergency : helpText}</p>
          <div class="suggestion-actions">
            {#if helpEmergency}
              <button class="suggestion-button primary" type="button" onclick={handleMinimize}>{t.ok}</button>
            {:else}
              <button class="suggestion-button primary" type="button" onclick={openHelpPage}>{t.helpQuestion}</button>
              <button class="suggestion-button danger" type="button" onclick={() => (helpEmergency = true)}>{t.helpEmergency}</button>
            {/if}
          </div>
        {:else if suggestion}
          <p class="suggestion-text">{suggestion.message}</p>
          <div class="suggestion-actions">
            <button class="suggestion-button primary" type="button" onclick={() => answerSuggestion("accept")}>
              {acceptLabel(suggestion)}
            </button>
            <button class="suggestion-button" type="button" onclick={() => answerSuggestion("later")}>{t.later}</button>
            <button class="suggestion-never" type="button" onclick={() => answerSuggestion("never")}>{t.never}</button>
          </div>
        {/if}
      </div>

      <div class="settings-panel" class:open={settingsOpen}>
        <div class="settings-row">
          <span class="settings-label">{t.launchAtLogin}</span>
          <button
            class="switch"
            class:on={settings.launch_at_startup}
            type="button"
            role="switch"
            aria-checked={settings.launch_at_startup}
            aria-label={t.launchAtLogin}
            onclick={toggleLaunchAtStartup}
          >
            <span class="switch-knob"></span>
          </button>
        </div>
        <div class="settings-row">
          <span class="settings-label">{t.shortcut}</span>
          <button
            class="keycap"
            class:capturing={capturingShortcut}
            type="button"
            aria-label={t.changeShortcut}
            onclick={startShortcutCapture}
          >
            {capturingShortcut ? t.pressKeys : shortcutLabel(settings.summon_shortcut)}
          </button>
        </div>
        <div class="settings-row">
          <span class="settings-label">{t.voiceWake}</span>
          <button
            class="switch"
            class:on={settings.voice_wake_enabled}
            type="button"
            role="switch"
            aria-checked={settings.voice_wake_enabled}
            aria-label={t.voiceWake}
            onclick={toggleVoiceWake}
          >
            <span class="switch-knob"></span>
          </button>
        </div>
        <div class="settings-row">
          <span class="settings-label">{t.language}</span>
          <div class="segmented" role="radiogroup" aria-label={t.language}>
            {#each [["en", "EN"], ["fr", "FR"]] as [code, label] (code)}
              <button
                class="segment"
                class:selected={settings.language === code}
                type="button"
                role="radio"
                aria-checked={settings.language === code}
                onclick={() => setLanguage(code as Language)}
              >
                {label}
              </button>
            {/each}
          </div>
        </div>
        <div class="settings-row">
          <span class="settings-label">{t.sounds}</span>
          <button
            class="switch"
            class:on={settings.sounds_enabled}
            type="button"
            role="switch"
            aria-checked={settings.sounds_enabled}
            aria-label={t.sounds}
            onclick={toggleSounds}
          >
            <span class="switch-knob"></span>
          </button>
        </div>
        <div class="settings-row">
          <span class="settings-label">{t.activity}</span>
          <button
            class="switch"
            class:on={settings.activity_enabled}
            type="button"
            role="switch"
            aria-checked={settings.activity_enabled}
            aria-label={t.activity}
            onclick={toggleActivity}
          >
            <span class="switch-knob"></span>
          </button>
        </div>
        <!-- Suggestions are built on the activity analysis: greyed out without it. -->
        <div class="settings-row" class:dimmed={!settings.activity_enabled}>
          <span class="settings-label">{t.suggestions}</span>
          <button
            class="switch"
            class:on={settings.suggestions_enabled && settings.activity_enabled}
            type="button"
            role="switch"
            aria-checked={settings.suggestions_enabled && settings.activity_enabled}
            aria-label={t.suggestions}
            disabled={!settings.activity_enabled}
            onclick={toggleSuggestions}
          >
            <span class="switch-knob"></span>
          </button>
        </div>
        <div class="settings-row ai-row">
          <span class="settings-label">
            {t.ai}
            {#if aiStatus.state === "downloading"}
              <span class="ai-note" title="{gigabytes(aiStatus.done)} / {gigabytes(aiStatus.total)}">· {aiPercent} %</span>
            {:else if aiStatus.state === "off" && !aiStatus.installed && aiStatus.size > 0}
              <span class="ai-note">· {gigabytes(aiStatus.size)}</span>
            {/if}
          </span>
          <div class="ai-controls">
            {#if aiStatus.state === "off" && aiStatus.installed}
              <button class="keycap" type="button" title={t.aiDeleteTitle} onclick={deleteAiFiles}>{t.aiDelete}</button>
            {/if}
            <button
              class="switch"
              class:on={settings.ai_enabled}
              type="button"
              role="switch"
              aria-checked={settings.ai_enabled}
              aria-label={t.ai}
              onclick={toggleAi}
            >
              <span class="switch-knob"></span>
            </button>
          </div>
          {#if aiStatus.state === "downloading"}
            <div class="ai-bar"><div class="ai-bar-fill" style="width: {aiPercent}%"></div></div>
          {/if}
        </div>
        <div class="settings-row">
          <span class="settings-label">{t.customize}</span>
          <button class="keycap" type="button" onclick={() => void invoke("open_customize_window")}>{t.open}</button>
        </div>
        <div class="settings-row">
          <span class="settings-label">{t.myCommands}</span>
          <button class="keycap" type="button" onclick={() => void invoke("open_commands_window")}>{t.manage}</button>
        </div>
        {#if shortcutError || shortcutNote || (settings.voice_wake_enabled && voiceError) || aiStatus.state === "error"}
          <p class="settings-hint">
            {shortcutError || shortcutNote || (settings.voice_wake_enabled && voiceError) || t.aiError}
          </p>
        {/if}
        <div class="settings-footer">
          <button class="settings-action" type="button" onclick={handleEraseMemory}>{t.eraseMemory}</button>
          {#if appVersion}<span class="version">{t.version} {appVersion}</span>{/if}
        </div>
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
      linear-gradient(160deg, rgba(var(--accent-rgb), var(--tint)), rgba(var(--accent-rgb), 0) 80%),
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
     matches SETTINGS_PANEL_HEIGHT (419) + the base 44px in src-tauri/src/lib.rs. */
  .pill.full.settings-open {
    height: 463px;
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
      transform 0.4s cubic-bezier(0.16, 1, 0.3, 1),
      opacity 0.2s ease;
  }

  .status.shifted {
    left: 18px;
    transform: translate(0, -50%);
  }

  .status.hidden {
    opacity: 0;
  }

  /* Typed prompt / listening / reply line — takes the whole bar while
     Mimo is handling a request (status and buttons fade out under it). */
  .prompt {
    position: absolute;
    inset: 0 18px;
    display: flex;
    align-items: center;
    gap: 0.55rem;
    min-width: 0;
  }

  .prompt-input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    padding: 0;
    background: transparent;
    color: #f5f5f7;
    caret-color: var(--accent);
    font-family:
      -apple-system,
      "SF Pro Display",
      "Segoe UI",
      Inter,
      sans-serif;
    font-size: 0.92rem;
    font-weight: 500;
    /* .bar and the page disable these; the prompt needs them back. */
    pointer-events: auto;
    user-select: text;
    -webkit-user-select: text;
  }

  .prompt-input::placeholder {
    color: rgba(245, 245, 247, 0.42);
  }

  .prompt-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ring-text {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    flex: 1;
  }

  .ring-time {
    font-variant-numeric: tabular-nums;
    font-weight: 700;
  }

  .ring-message {
    overflow: hidden;
    text-overflow: ellipsis;
    color: rgba(245, 245, 247, 0.75);
  }

  .stop-button {
    flex-shrink: 0;
    border: none;
    border-radius: 999px;
    padding: 4px 12px;
    background: #ff9f0a;
    color: #1c1c1e;
    font-family:
      -apple-system,
      "SF Pro Display",
      "Segoe UI",
      Inter,
      sans-serif;
    font-size: 0.78rem;
    font-weight: 700;
    cursor: pointer;
    pointer-events: auto;
    transition: transform 0.1s ease, filter 0.15s ease;
  }

  .stop-button:hover {
    filter: brightness(1.1);
  }

  .stop-button:active {
    transform: scale(0.94);
  }

  .dot.running.alarm {
    background: #ff9f0a;
    box-shadow: 0 0 10px rgba(255, 159, 10, 0.85);
    animation: dot-pulse 0.8s ease-in-out infinite;
  }

  .prompt-text.muted {
    color: rgba(245, 245, 247, 0.6);
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
  /* Grows the same way as .settings-open, by TRANSLATION_DRAWER_HEIGHT. */
  .pill.full.drawer-open {
    height: calc(44px + 170px);
    border-radius: 26px;
    /* Text to read: solid, nothing showing through. */
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.08), rgba(255, 255, 255, 0) 38%),
      linear-gradient(160deg, #2a2a31, #111115);
  }

  /* Grows by SUGGESTION_DRAWER_HEIGHT, like the translation card. */
  .pill.full.suggestion-open {
    height: calc(44px + 118px);
    border-radius: 26px;
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.08), rgba(255, 255, 255, 0) 38%),
      linear-gradient(160deg, #2a2a31, #111115);
  }

  .suggestion-panel {
    flex: 0 0 0;
    width: 100%;
    box-sizing: border-box;
    overflow: hidden;
    opacity: 0;
    padding: 0 18px;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 12px;
    pointer-events: none;
    transition:
      flex-basis 0.4s cubic-bezier(0.16, 1, 0.3, 1),
      opacity 0.3s ease;
    font-family:
      -apple-system,
      "SF Pro Display",
      "Segoe UI",
      Inter,
      sans-serif;
  }

  .suggestion-panel.open {
    flex-basis: 118px;
    opacity: 1;
    pointer-events: auto;
  }

  .suggestion-text {
    margin: 0;
    font-size: 0.86rem;
    line-height: 1.35;
    color: #f5f5f7;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .suggestion-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .suggestion-button {
    border: none;
    border-radius: 999px;
    padding: 5px 13px;
    background: rgba(255, 255, 255, 0.12);
    color: #f5f5f7;
    font: inherit;
    font-size: 0.76rem;
    font-weight: 600;
    cursor: pointer;
    transition: transform 0.1s ease, filter 0.15s ease, background 0.15s ease;
  }

  .suggestion-button:hover {
    background: rgba(255, 255, 255, 0.18);
  }

  .suggestion-button.primary {
    background: var(--accent);
    color: var(--on-accent);
  }

  .suggestion-button.primary:hover {
    filter: brightness(1.1);
  }

  .suggestion-button:active {
    transform: scale(0.95);
  }

  .suggestion-never {
    margin-left: auto;
    border: none;
    padding: 0;
    background: none;
    color: rgba(245, 245, 247, 0.45);
    font: inherit;
    font-size: 0.72rem;
    cursor: pointer;
  }

  .suggestion-never:hover {
    color: rgba(245, 245, 247, 0.75);
  }

  .dot.running.suggesting {
    background: var(--accent);
    box-shadow: 0 0 10px rgba(var(--accent-rgb), 0.85);
  }

  .translation-panel {
    flex: 0 0 0;
    width: 100%;
    box-sizing: border-box;
    overflow: hidden;
    opacity: 0;
    padding: 0 18px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    pointer-events: none;
    transition:
      flex-basis 0.4s cubic-bezier(0.16, 1, 0.3, 1),
      opacity 0.3s ease;
    font-family:
      -apple-system,
      "SF Pro Display",
      "Segoe UI",
      Inter,
      sans-serif;
  }

  .translation-panel.open {
    flex-basis: 170px;
    opacity: 1;
    pointer-events: auto;
  }

  .translation-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .lang-chip {
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.06em;
    color: rgba(245, 245, 247, 0.5);
  }

  .copy-button {
    border: none;
    border-radius: 999px;
    padding: 4px 11px;
    background: rgba(var(--accent-rgb), 0.2);
    color: var(--accent-text);
    font: inherit;
    font-size: 0.74rem;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.15s ease, color 0.15s ease;
  }

  .copy-button:hover {
    background: rgba(var(--accent-rgb), 0.3);
  }

  .copy-button.copied {
    background: rgba(50, 215, 75, 0.2);
    color: #7ee891;
  }

  .translation-text {
    margin: 0;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    font-size: 0.95rem;
    font-weight: 500;
    line-height: 1.35;
    color: #f5f5f7;
    user-select: text;
    -webkit-user-select: text;
  }

  .translation-original {
    margin: 0 0 14px;
    font-size: 0.74rem;
    line-height: 1.3;
    color: rgba(245, 245, 247, 0.45);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    user-select: text;
    -webkit-user-select: text;
  }

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
    flex-basis: 419px;
    opacity: 1;
    pointer-events: auto;
  }

  .settings-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }

  /* The download bar runs along the bottom of the "Local AI" row. */
  .ai-row {
    position: relative;
  }

  .ai-note {
    color: rgba(245, 245, 247, 0.45);
    font-weight: 400;
  }

  .ai-controls {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .ai-bar {
    position: absolute;
    left: 0;
    right: 0;
    bottom: -5px;
    height: 3px;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.1);
    overflow: hidden;
  }

  .ai-bar-fill {
    height: 100%;
    border-radius: 999px;
    background: var(--accent);
    transition: width 0.25s ease;
  }

  .settings-row.dimmed {
    opacity: 0.4;
  }

  .switch:disabled {
    cursor: default;
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
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.4);
    transition: transform 0.2s cubic-bezier(0.16, 1, 0.3, 1);
  }

  .switch.on .switch-knob {
    transform: translateX(15px);
  }

  .keycap {
    min-width: 36px;
    border: none;
    border-radius: 6px;
    padding: 3px 9px;
    background: rgba(255, 255, 255, 0.12);
    box-shadow: 0 1px 0 rgba(255, 255, 255, 0.12) inset, 0 1px 2px rgba(0, 0, 0, 0.35);
    color: #f5f5f7;
    font-family:
      -apple-system,
      "SF Pro Display",
      "Segoe UI",
      Inter,
      sans-serif;
    font-size: 0.76rem;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.15s ease;
  }

  .keycap:hover {
    background: rgba(255, 255, 255, 0.2);
  }

  .keycap.capturing {
    background: rgba(var(--accent-rgb), 0.24);
    color: #7ee891;
  }

  .segmented {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: 7px;
    background: rgba(255, 255, 255, 0.1);
  }

  .segment {
    border: none;
    border-radius: 5px;
    padding: 2px 9px;
    background: transparent;
    color: rgba(245, 245, 247, 0.6);
    font-family:
      -apple-system,
      "SF Pro Display",
      "Segoe UI",
      Inter,
      sans-serif;
    font-size: 0.72rem;
    font-weight: 600;
    cursor: pointer;
    transition:
      background 0.15s ease,
      color 0.15s ease;
  }

  .segment.selected {
    background: rgba(255, 255, 255, 0.22);
    color: #f5f5f7;
  }

  .settings-hint {
    margin: -2px 0 0;
    color: #ff9f97;
    font-family:
      -apple-system,
      "SF Pro Display",
      "Segoe UI",
      Inter,
      sans-serif;
    font-size: 0.7rem;
    line-height: 1.3;
    user-select: text;
    -webkit-user-select: text;
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

  .settings-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }

  .version {
    color: rgba(245, 245, 247, 0.38);
    font-family:
      -apple-system,
      "SF Pro Display",
      "Segoe UI",
      Inter,
      sans-serif;
    font-size: 0.72rem;
    font-variant-numeric: tabular-nums;
  }

  .suggestion-button.danger {
    background: rgba(255, 69, 58, 0.18);
    color: #ff6961;
  }

  .suggestion-button.danger:hover {
    background: rgba(255, 69, 58, 0.3);
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
    background: var(--accent);
    box-shadow: 0 0 8px rgba(var(--accent-rgb), 0.7);
  }

  .dot.listening {
    background: var(--accent);
    box-shadow: 0 0 10px rgba(var(--accent-rgb), 0.8);
    animation: dot-pulse 1.1s ease-in-out infinite;
  }

  .dot.thinking {
    animation: dot-pulse 0.7s ease-in-out infinite;
  }

  .dot.failed {
    background: #ff453a;
    box-shadow: 0 0 8px rgba(255, 69, 58, 0.7);
  }

  @keyframes dot-pulse {
    0%,
    100% {
      transform: scale(1);
      opacity: 1;
    }
    50% {
      transform: scale(0.7);
      opacity: 0.6;
    }
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
