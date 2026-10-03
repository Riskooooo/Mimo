<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";

  type Lang = "fr" | "en";
  type Level = "good" | "info" | "warning" | "critical";
  type Advice = { level: Level; title: string; detail: string };
  type Snapshot = {
    cpu_name: string;
    cpu_cores: number;
    cpu_usage: number;
    memory_used: number;
    memory_total: number;
    disks: { mount: string; used: number; total: number }[];
    uptime_secs: number;
    temperature_c: number | null;
    gpu: { name: string; temperature_c: number | null; usage: number | null } | null;
    battery: { percent: number; charging: boolean } | null;
    top_processes: { name: string; memory: number; cpu: number }[];
    process_count: number;
  };
  type NotificationItem = { app: string; title: string; body: string; time: number };
  type Reminder = { id: number; kind: "reminder" | "alarm"; due: number; message: string | null };
  type Content =
    | { kind: "diagnostic"; lang: Lang; snapshot: Snapshot; advice: Advice[]; summary: string }
    | { kind: "notifications"; lang: Lang; items: NotificationItem[]; error: string | null }
    | { kind: "reminders"; lang: Lang; items: Reminder[] }
    | { kind: "tasks"; lang: Lang; items: Task[]; summary: string }
    | {
        kind: "activity";
        lang: Lang;
        period: "today" | "week";
        total_secs: number;
        apps: AppUsage[];
        chart: number[];
        chart_start: number;
        summary: string;
      };
  type AppUsage = { app: string; label: string; secs: number };
  // due_date "2026-10-03", due_time "14:00:00" (local).
  type Task = { id: number; title: string; due_date: string | null; due_time: string | null };

  const TEXT = {
    fr: {
      diagnostic: "Diagnostic",
      notifications: "Notifications",
      reminders: "Rappels",
      cpu: "Processeur",
      memory: "Mémoire",
      temperature: "Température",
      gpu: "Carte graphique",
      advice: "Conseils",
      storage: "Stockage",
      apps: "Applications gourmandes",
      free: "libres",
      refresh: "Actualiser",
      uptime: "Allumé depuis",
      processes: "processus",
      battery: "Batterie",
      charging: "en charge",
      noNotifications: "Aucune notification",
      noNotificationsHint: "Le centre de notifications de Windows est vide.",
      notificationsError: "Impossible de lire les notifications",
      noReminders: "Aucun rappel",
      noRemindersHint: "Dis « rappelle-moi dans 10 minutes de… » ou écris-en un ci-dessous.",
      newReminder: "Nouveau rappel… ex. « appeler maman demain à 18h »",
      today: "Aujourd'hui",
      tomorrow: "Demain",
      alarm: "Réveil",
      reminder: "Rappel",
      justNow: "à l'instant",
      minutesAgo: (n: number) => `il y a ${n} min`,
      hoursAgo: (n: number) => `il y a ${n} h`,
      daysAgo: (n: number) => `il y a ${n} j`,
      close: "Fermer",
      delete: "Supprimer",
      na: "n/d",
      gb: "Go",
      day: "j",
      tasks: "Tâches",
      overdue: "En retard",
      upcoming: "À venir",
      noDate: "Sans date",
      newTask: "Nouvelle tâche… ex. « acheter du pain demain 18h »",
      allDone: "Rien à faire",
      allDoneHint: "Dis « ajoute une tâche : … » ou écris-la ci-dessous.",
      at: "à",
      complete: "Terminer",
      edit: "Modifier",
      save: "Enregistrer",
      cancel: "Annuler",
      clearDate: "Retirer la date",
      activity: "Activité",
      byHour: "Par heure",
      byDay: "Par jour",
      topApps: "Applications",
      noActivity: "Rien pour l'instant",
      noActivityHint: "Mimo apprend au fil de ton utilisation — reviens dans un moment.",
    },
    en: {
      diagnostic: "Check-up",
      notifications: "Notifications",
      reminders: "Reminders",
      cpu: "CPU",
      memory: "Memory",
      temperature: "Temperature",
      gpu: "Graphics",
      advice: "Advice",
      storage: "Storage",
      apps: "Heaviest apps",
      free: "free",
      refresh: "Refresh",
      uptime: "Up for",
      processes: "processes",
      battery: "Battery",
      charging: "charging",
      noNotifications: "No notifications",
      noNotificationsHint: "Windows' notification center is empty.",
      notificationsError: "Couldn't read the notifications",
      noReminders: "No reminders",
      noRemindersHint: "Say “remind me in 10 minutes to…” or type one below.",
      newReminder: "New reminder… e.g. “call mom tomorrow at 6pm”",
      today: "Today",
      tomorrow: "Tomorrow",
      alarm: "Alarm",
      reminder: "Reminder",
      justNow: "just now",
      minutesAgo: (n: number) => `${n} min ago`,
      hoursAgo: (n: number) => `${n} h ago`,
      daysAgo: (n: number) => `${n} d ago`,
      close: "Close",
      delete: "Delete",
      na: "n/a",
      gb: "GB",
      day: "d",
      tasks: "Tasks",
      overdue: "Overdue",
      upcoming: "Upcoming",
      noDate: "No date",
      newTask: "New task… e.g. “buy bread tomorrow 6pm”",
      allDone: "All done",
      allDoneHint: "Say “add a task: …” or type one below.",
      at: "at",
      complete: "Complete",
      edit: "Edit",
      save: "Save",
      cancel: "Cancel",
      clearDate: "Remove the date",
      activity: "Activity",
      byHour: "By hour",
      byDay: "By day",
      topApps: "Apps",
      noActivity: "Nothing yet",
      noActivityHint: "Mimo learns as you use your PC — check back in a while.",
    },
  };

  let content = $state<Content | null>(null);
  // Bumped on every show so the entrance animation replays.
  let showId = $state(0);
  let refreshing = $state(false);

  const t = $derived(TEXT[content?.lang ?? "fr"]);

  const GB = 1024 ** 3;
  const gb = (bytes: number) => (bytes / GB).toFixed(bytes >= 100 * GB ? 0 : 1);
  const pct = (used: number, total: number) => (total ? (used / total) * 100 : 0);

  // iOS system colors.
  const GREEN = "#32d74b";
  const YELLOW = "#ffd60a";
  const ORANGE = "#ff9f0a";
  const RED = "#ff453a";
  const BLUE = "#0a84ff";

  function loadColor(percent: number) {
    if (percent >= 90) return RED;
    if (percent >= 75) return ORANGE;
    if (percent >= 60) return YELLOW;
    return GREEN;
  }

  function heatColor(celsius: number) {
    if (celsius >= 90) return RED;
    if (celsius >= 80) return ORANGE;
    if (celsius >= 70) return YELLOW;
    return GREEN;
  }

  const levelColor: Record<Level, string> = { good: GREEN, info: BLUE, warning: ORANGE, critical: RED };

  // "2 h 05" / "45 min", like the pill's answers.
  function duration(secs: number) {
    const minutes = Math.floor(secs / 60);
    const h = Math.floor(minutes / 60);
    const m = minutes % 60;
    if (h === 0) return `${m} min`;
    return `${h} h ${m.toString().padStart(2, "0")}`;
  }

  // Chart bar labels: hours of today, or the last 7 days' initials.
  function chartLabel(index: number) {
    if (content?.kind !== "activity") return "";
    if (content.period === "today") return index % 6 === 0 ? `${index}h` : "";
    const day = new Date((content.chart_start + index * 86400) * 1000);
    return day.toLocaleDateString(content.lang === "en" ? "en-US" : "fr-FR", { weekday: "narrow" });
  }

  function uptime(secs: number) {
    const days = Math.floor(secs / 86400);
    const hours = Math.floor((secs % 86400) / 3600);
    const minutes = Math.floor((secs % 3600) / 60);
    if (days > 0) return `${days} ${t.day} ${hours} h`;
    if (hours > 0) return `${hours} h ${minutes.toString().padStart(2, "0")}`;
    return `${minutes} min`;
  }

  function ago(unix: number) {
    const minutes = Math.max(0, Math.round((Date.now() / 1000 - unix) / 60));
    if (minutes < 1) return t.justNow;
    if (minutes < 60) return t.minutesAgo(minutes);
    if (minutes < 60 * 24) return t.hoursAgo(Math.round(minutes / 60));
    return t.daysAgo(Math.round(minutes / 1440));
  }

  function clock(unix: number) {
    const d = new Date(unix * 1000);
    return d.toLocaleTimeString(content?.lang === "en" ? "en-US" : "fr-FR", { hour: "2-digit", minute: "2-digit" });
  }

  function dayLabel(unix: number) {
    const d = new Date(unix * 1000);
    const today = new Date();
    const tomorrow = new Date();
    tomorrow.setDate(today.getDate() + 1);
    if (d.toDateString() === today.toDateString()) return t.today;
    if (d.toDateString() === tomorrow.toDateString()) return t.tomorrow;
    return d.toLocaleDateString(content?.lang === "en" ? "en-US" : "fr-FR", { weekday: "long", day: "numeric", month: "long" });
  }

  // Task due moments, re-evaluated every half minute so "overdue" turns
  // red on time while the panel is open.
  let now = $state(Date.now());

  function taskMoment(task: Task): number | null {
    if (!task.due_date) return null;
    const [y, m, d] = task.due_date.split("-").map(Number);
    if (task.due_time) {
      const [hh, mm] = task.due_time.split(":").map(Number);
      return new Date(y, m - 1, d, hh, mm).getTime();
    }
    // No time: due by the end of that day.
    return new Date(y, m - 1, d, 23, 59, 59).getTime();
  }

  function taskDay(task: Task): Date | null {
    if (!task.due_date) return null;
    const [y, m, d] = task.due_date.split("-").map(Number);
    return new Date(y, m - 1, d);
  }

  function isOverdue(task: Task) {
    const moment = taskMoment(task);
    return moment !== null && moment < now;
  }

  function isToday(task: Task) {
    const day = taskDay(task);
    return day !== null && day.toDateString() === new Date(now).toDateString();
  }

  function taskDueLabel(task: Task) {
    const day = taskDay(task);
    if (!day) return "";
    const locale = content?.lang === "en" ? "en-US" : "fr-FR";
    const label = dayLabel(day.getTime() / 1000);
    if (!task.due_time) return label;
    const [hh, mm] = task.due_time.split(":").map(Number);
    const time = new Date(2000, 0, 1, hh, mm).toLocaleTimeString(locale, { hour: "2-digit", minute: "2-digit" });
    return `${label} ${t.at} ${time}`;
  }

  type TaskSection = { key: string; title: string; tone: "red" | "blue" | "gray"; items: Task[] };

  const taskSections = $derived.by((): TaskSection[] => {
    if (content?.kind !== "tasks") return [];
    const overdue: Task[] = [];
    const today: Task[] = [];
    const upcoming: Task[] = [];
    const undated: Task[] = [];
    for (const task of content.items) {
      if (isOverdue(task)) overdue.push(task);
      else if (isToday(task)) today.push(task);
      else if (task.due_date) upcoming.push(task);
      else undated.push(task);
    }
    return [
      { key: "overdue", title: t.overdue, tone: "red" as const, items: overdue },
      { key: "today", title: t.today, tone: "blue" as const, items: today },
      { key: "upcoming", title: t.upcoming, tone: "blue" as const, items: upcoming },
      { key: "undated", title: t.noDate, tone: "gray" as const, items: undated },
    ].filter((section) => section.items.length > 0);
  });

  // Ticked tasks get a short "done" animation before leaving the list.
  let completing = $state<number[]>([]);
  let newTask = $state("");

  async function completeTask(id: number) {
    if (completing.includes(id)) return;
    completing = [...completing, id];
    await new Promise((resolve) => setTimeout(resolve, 450));
    content = await invoke<Content>("complete_task", { id });
    completing = completing.filter((c) => c !== id);
  }

  async function addTask(event: KeyboardEvent) {
    if (event.key !== "Enter" || !newTask.trim()) return;
    const text = newTask.trim();
    newTask = "";
    content = await invoke<Content>("add_task_text", { text });
  }

  // Editing a task in place: its title, day and time.
  let editingId = $state<number | null>(null);
  let editTitle = $state("");
  let editDate = $state("");
  let editTime = $state("");

  function startEdit(task: Task) {
    editingId = task.id;
    editTitle = task.title;
    editDate = task.due_date ?? "";
    editTime = task.due_time?.slice(0, 5) ?? "";
  }

  function cancelEdit() {
    editingId = null;
  }

  function clearEditDate() {
    editDate = "";
    editTime = "";
  }

  async function saveEdit() {
    if (editingId === null || !editTitle.trim()) return;
    const id = editingId;
    editingId = null;
    content = await invoke<Content>("update_task", {
      id,
      title: editTitle.trim(),
      date: editDate || null,
      time: (editDate && editTime) || null,
    });
  }

  function editKeydown(event: KeyboardEvent) {
    if (event.key === "Enter") {
      event.preventDefault();
      void saveEdit();
    } else if (event.key === "Escape") {
      // Leave the edit, not the whole panel.
      event.stopPropagation();
      cancelEdit();
    }
  }

  function autofocus(node: HTMLInputElement) {
    node.focus();
    node.select();
  }

  let newReminder = $state("");
  let reminderError = $state("");

  async function addReminder(event: KeyboardEvent) {
    if (event.key !== "Enter" || !newReminder.trim() || content?.kind !== "reminders") return;
    try {
      content = await invoke<Content>("add_reminder_text", { text: newReminder.trim(), lang: content.lang });
      newReminder = "";
      reminderError = "";
    } catch (err) {
      reminderError = String(err);
    }
  }

  function show(next: Content) {
    content = next;
    editingId = null;
    reminderError = "";
    showId += 1;
  }

  async function close() {
    await invoke("close_panel");
  }

  async function refresh() {
    if (content?.kind !== "diagnostic" || refreshing) return;
    refreshing = true;
    try {
      const next = await invoke<Content | null>("refresh_diagnostic", { lang: content.lang });
      if (next) show(next);
    } finally {
      refreshing = false;
    }
  }

  async function removeReminder(id: number) {
    if (content?.kind !== "reminders") return;
    content = await invoke<Content>("remove_reminder", { id, lang: content.lang });
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") void close();
  }

  onMount(() => {
    void invoke<Content | null>("get_panel_content").then((current) => {
      if (current) show(current);
    });
    const unlisten = listen<Content>("mimo://panel", (event) => show(event.payload));
    const clock = setInterval(() => (now = Date.now()), 30_000);
    return () => {
      void unlisten.then((fn) => fn());
      clearInterval(clock);
    };
  });

  // Ring gauge geometry.
  const R = 30;
  const C = 2 * Math.PI * R;
</script>

<svelte:window onkeydown={handleKeydown} />

{#snippet ring(value: number, max: number, color: string, display: string, unit: string, label: string, sub: string)}
  <div class="gauge">
    <svg viewBox="0 0 76 76" class="ring" aria-hidden="true">
      <circle cx="38" cy="38" r={R} class="ring-track" />
      <circle
        cx="38"
        cy="38"
        r={R}
        class="ring-value"
        stroke={color}
        stroke-dasharray={C}
        style={`--target: ${C * (1 - Math.min(Math.max(value / max, 0), 1))}; --full: ${C}`}
      />
    </svg>
    <div class="gauge-value">
      <span class="gauge-number">{display}</span><span class="gauge-unit">{unit}</span>
    </div>
    <span class="gauge-label">{label}</span>
    <span class="gauge-sub">{sub}</span>
  </div>
{/snippet}

<main class="window">
  {#if content}
    {#key showId}
      <section class="sheet" in:fly={{ y: 14, duration: 420, easing: cubicOut, opacity: 0 }}>
        <header class="header" data-tauri-drag-region>
          <div class="header-text" data-tauri-drag-region>
            <h1 data-tauri-drag-region>{t[content.kind]}</h1>
            {#if content.kind === "diagnostic" || content.kind === "tasks" || content.kind === "activity"}
              <p class="subtitle" data-tauri-drag-region>{content.summary}</p>
            {/if}
          </div>
          <button class="close" type="button" aria-label={t.close} onclick={close}>
            <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
              <line x1="2" y1="2" x2="10" y2="10" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
              <line x1="10" y1="2" x2="2" y2="10" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
            </svg>
          </button>
        </header>

        <div class="scroll">
          {#if content.kind === "diagnostic"}
            {@const s = content.snapshot}
            {@const memPct = pct(s.memory_used, s.memory_total)}
            <div class="card gauges">
              {@render ring(s.cpu_usage, 100, loadColor(s.cpu_usage), s.cpu_usage.toFixed(0), "%", t.cpu, `${s.cpu_cores} threads`)}
              {@render ring(memPct, 100, loadColor(memPct), memPct.toFixed(0), "%", t.memory, `${gb(s.memory_used)} / ${gb(s.memory_total)} ${t.gb}`)}
              {#if s.temperature_c !== null}
                {@render ring(s.temperature_c - 20, 80, heatColor(s.temperature_c), s.temperature_c.toFixed(0), "°", t.temperature, "PC")}
              {:else}
                {@render ring(0, 1, GREEN, t.na, "", t.temperature, "PC")}
              {/if}
              {#if s.gpu && s.gpu.temperature_c !== null}
                {@render ring(s.gpu.temperature_c - 20, 80, heatColor(s.gpu.temperature_c), s.gpu.temperature_c.toFixed(0), "°", t.gpu, `${(s.gpu.usage ?? 0).toFixed(0)} %`)}
              {:else if s.battery}
                {@render ring(s.battery.percent, 100, s.battery.percent < 20 ? RED : GREEN, `${s.battery.percent}`, "%", t.battery, s.battery.charging ? t.charging : "")}
              {/if}
            </div>

            <h2>{t.advice}</h2>
            <div class="card list">
              {#each content.advice as item, i (i)}
                <div class="row advice">
                  <span class="badge" style={`--c: ${levelColor[item.level]}`}>
                    {#if item.level === "good"}
                      <svg viewBox="0 0 12 12" width="11" height="11" aria-hidden="true"><polyline points="2.5,6.4 5,8.8 9.6,3.4" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg>
                    {:else if item.level === "info"}
                      <svg viewBox="0 0 12 12" width="11" height="11" aria-hidden="true"><circle cx="6" cy="3" r="1.1" fill="currentColor" /><line x1="6" y1="5.4" x2="6" y2="9.6" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" /></svg>
                    {:else}
                      <svg viewBox="0 0 12 12" width="11" height="11" aria-hidden="true"><line x1="6" y1="2.4" x2="6" y2="6.8" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" /><circle cx="6" cy="9.3" r="1.1" fill="currentColor" /></svg>
                    {/if}
                  </span>
                  <div class="row-text">
                    <span class="row-title">{item.title}</span>
                    <span class="row-detail">{item.detail}</span>
                  </div>
                </div>
              {/each}
            </div>

            <h2>{t.storage}</h2>
            <div class="card list">
              {#each s.disks as disk (disk.mount)}
                {@const used = pct(disk.used, disk.total)}
                <div class="row column">
                  <div class="row-line">
                    <span class="row-title">{disk.mount}</span>
                    <span class="row-detail">{gb(disk.total - disk.used)} {t.gb} {t.free} · {gb(disk.total)} {t.gb}</span>
                  </div>
                  <div class="bar"><span style={`width: ${used}%; background: ${loadColor(used)}`}></span></div>
                </div>
              {/each}
            </div>

            <h2>{t.apps}</h2>
            <div class="card list">
              {#each s.top_processes as proc (proc.name)}
                <div class="row column">
                  <div class="row-line">
                    <span class="row-title">{proc.name.replace(/\.exe$/i, "")}</span>
                    <span class="row-detail">{gb(proc.memory)} {t.gb} · {proc.cpu.toFixed(0)} % CPU</span>
                  </div>
                  <div class="bar"><span style={`width: ${pct(proc.memory, s.memory_total)}%; background: ${BLUE}`}></span></div>
                </div>
              {/each}
            </div>

            <footer class="footer">
              <span>{s.cpu_name}</span>
              <span>{t.uptime} {uptime(s.uptime_secs)} · {s.process_count} {t.processes}</span>
              <button class="pill-button" type="button" onclick={refresh} disabled={refreshing}>
                <svg class:spinning={refreshing} viewBox="0 0 12 12" width="11" height="11" aria-hidden="true">
                  <path d="M10 6a4 4 0 1 1-1.2-2.85" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
                  <polyline points="9.4,1.4 9.2,3.4 7.2,3.3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
                </svg>
                {t.refresh}
              </button>
            </footer>
          {:else if content.kind === "notifications"}
            {#if content.error}
              <div class="empty">
                <span class="empty-icon" style={`--c: ${RED}`}>!</span>
                <span class="empty-title">{t.notificationsError}</span>
                <span class="empty-hint">{content.error}</span>
              </div>
            {:else if content.items.length === 0}
              <div class="empty">
                <svg class="empty-glyph" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3a6 6 0 0 0-6 6v3.2L4.4 15a1 1 0 0 0 .9 1.5h13.4a1 1 0 0 0 .9-1.5L18 12.2V9a6 6 0 0 0-6-6Z" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" /><path d="M9.8 19a2.3 2.3 0 0 0 4.4 0" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>
                <span class="empty-title">{t.noNotifications}</span>
                <span class="empty-hint">{t.noNotificationsHint}</span>
              </div>
            {:else}
              {#each content.items as item, i (i)}
                <article class="card notification" in:fly={{ y: 8, duration: 300, delay: 40 * i, easing: cubicOut }}>
                  <div class="row-line">
                    <span class="app-tag">{item.app}</span>
                    <span class="row-detail">{ago(item.time)}</span>
                  </div>
                  <span class="row-title">{item.title}</span>
                  {#if item.body}<span class="row-detail body">{item.body}</span>{/if}
                </article>
              {/each}
            {/if}
          {:else if content.kind === "reminders"}
            {#if content.items.length === 0}
              <div class="empty">
                <svg class="empty-glyph" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="13" r="7" fill="none" stroke="currentColor" stroke-width="1.6" /><polyline points="12,9.5 12,13 14.5,14.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /><line x1="5" y1="4.5" x2="3" y2="6.5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /><line x1="19" y1="4.5" x2="21" y2="6.5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>
                <span class="empty-title">{t.noReminders}</span>
                <span class="empty-hint">{t.noRemindersHint}</span>
              </div>
            {:else}
              <div class="card list">
                {#each content.items as item (item.id)}
                  <div class="row reminder">
                    <span class="badge" style={`--c: ${item.kind === "alarm" ? ORANGE : BLUE}`}>
                      {#if item.kind === "alarm"}
                        <svg viewBox="0 0 12 12" width="11" height="11" aria-hidden="true"><circle cx="6" cy="6.6" r="3.6" fill="none" stroke="currentColor" stroke-width="1.4" /><polyline points="6,4.8 6,6.7 7.2,7.5" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" /></svg>
                      {:else}
                        <svg viewBox="0 0 12 12" width="11" height="11" aria-hidden="true"><path d="M6 1.8a3 3 0 0 0-3 3v1.7L2.2 8.2h7.6L9 6.5V4.8a3 3 0 0 0-3-3Z" fill="currentColor" /><circle cx="6" cy="10" r="1" fill="currentColor" /></svg>
                      {/if}
                    </span>
                    <div class="row-text">
                      <span class="time">{clock(item.due)}</span>
                      <span class="row-detail">{dayLabel(item.due)} · {item.message ?? (item.kind === "alarm" ? t.alarm : t.reminder)}</span>
                    </div>
                    <button class="delete" type="button" aria-label={t.delete} onclick={() => removeReminder(item.id)}>
                      <svg viewBox="0 0 12 12" width="11" height="11" aria-hidden="true"><line x1="2.5" y1="6" x2="9.5" y2="6" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" /></svg>
                    </button>
                  </div>
                {/each}
              </div>
            {/if}
          {:else if content.kind === "tasks"}
            {#if content.items.length === 0}
              <div class="empty">
                <svg class="empty-glyph" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9" fill="none" stroke="currentColor" stroke-width="1.6" /><polyline points="7.8,12.4 10.6,15.2 16.4,9" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
                <span class="empty-title">{t.allDone}</span>
                <span class="empty-hint">{t.allDoneHint}</span>
              </div>
            {/if}
            {#each taskSections as section (section.key)}
              <h2 class:danger={section.tone === "red"}>{section.title}</h2>
              <div class="card list">
                {#each section.items as task (task.id)}
                  <div
                    class="row task"
                    class:done={completing.includes(task.id)}
                    class:editing={editingId === task.id}
                    out:fly={{ x: 24, duration: 260, easing: cubicOut }}
                  >
                    {#if editingId === task.id}
                      <input
                        class="edit-title"
                        type="text"
                        spellcheck="false"
                        autocomplete="off"
                        bind:value={editTitle}
                        onkeydown={editKeydown}
                        use:autofocus
                      />
                      <div class="edit-when">
                        <input type="date" bind:value={editDate} onkeydown={editKeydown} />
                        <input type="time" bind:value={editTime} disabled={!editDate} onkeydown={editKeydown} />
                        {#if editDate}
                          <button class="clear-date" type="button" aria-label={t.clearDate} title={t.clearDate} onclick={clearEditDate}>
                            <svg viewBox="0 0 12 12" width="9" height="9" aria-hidden="true"><line x1="3" y1="3" x2="9" y2="9" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /><line x1="9" y1="3" x2="3" y2="9" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>
                          </button>
                        {/if}
                      </div>
                      <div class="edit-actions">
                        <button class="text-button" type="button" onclick={cancelEdit}>{t.cancel}</button>
                        <button class="text-button primary" type="button" disabled={!editTitle.trim()} onclick={saveEdit}>{t.save}</button>
                      </div>
                    {:else}
                    <button
                      class="check"
                      class:red={section.tone === "red"}
                      type="button"
                      aria-label={t.complete}
                      onclick={() => completeTask(task.id)}
                    >
                      <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true"><polyline points="2.6,6.3 5,8.6 9.4,3.6" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg>
                    </button>
                    <div class="row-text">
                      <span class="row-title task-title">{task.title}</span>
                      {#if task.due_date}
                        <span class="row-detail" class:late={section.tone === "red"}>{taskDueLabel(task)}</span>
                      {/if}
                    </div>
                    <button class="edit" type="button" aria-label={t.edit} title={t.edit} onclick={() => startEdit(task)}>
                      <svg viewBox="0 0 12 12" width="11" height="11" aria-hidden="true"><path d="M7.6 2.4l2 2L4.4 9.6 2 10l.4-2.4z" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" /></svg>
                    </button>
                    {/if}
                  </div>
                {/each}
              </div>
            {/each}
          {:else if content.kind === "activity"}
            {#if content.total_secs < 60}
              <div class="empty">
                <svg class="empty-glyph" viewBox="0 0 24 24" aria-hidden="true"><rect x="4" y="12" width="3.4" height="8" rx="1" fill="none" stroke="currentColor" stroke-width="1.6" /><rect x="10.3" y="7" width="3.4" height="13" rx="1" fill="none" stroke="currentColor" stroke-width="1.6" /><rect x="16.6" y="4" width="3.4" height="16" rx="1" fill="none" stroke="currentColor" stroke-width="1.6" /></svg>
                <span class="empty-title">{t.noActivity}</span>
                <span class="empty-hint">{t.noActivityHint}</span>
              </div>
            {:else}
              {@const peak = Math.max(...content.chart, 1)}
              <h2>{content.period === "today" ? t.byHour : t.byDay}</h2>
              <div class="card chart" class:week={content.period === "week"}>
                {#each content.chart as minutes, i (i)}
                  <div class="chart-column" title={duration(minutes * 60)}>
                    <div class="chart-track">
                      <span class="chart-bar" style={`height: ${(minutes / peak) * 100}%; animation-delay: ${i * 18}ms`}></span>
                    </div>
                    <span class="chart-label">{chartLabel(i)}</span>
                  </div>
                {/each}
              </div>

              <h2>{t.topApps}</h2>
              <div class="card list">
                {#each content.apps.slice(0, 8) as usage (usage.app)}
                  <div class="row column">
                    <div class="row-line">
                      <span class="row-title">{usage.label}</span>
                      <span class="row-detail">{duration(usage.secs)}</span>
                    </div>
                    <div class="bar"><span style={`width: ${(usage.secs / content.apps[0].secs) * 100}%; background: ${BLUE}`}></span></div>
                  </div>
                {/each}
              </div>
            {/if}
          {/if}
        </div>
        {#if content.kind === "tasks"}
          <div class="add-task">
            <svg viewBox="0 0 12 12" width="14" height="14" aria-hidden="true"><line x1="6" y1="2" x2="6" y2="10" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /><line x1="2" y1="6" x2="10" y2="6" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>
            <input
              type="text"
              placeholder={t.newTask}
              spellcheck="false"
              autocomplete="off"
              bind:value={newTask}
              onkeydown={addTask}
            />
          </div>
        {:else if content.kind === "reminders"}
          {#if reminderError}
            <p class="add-error">{reminderError}</p>
          {/if}
          <div class="add-task">
            <svg viewBox="0 0 12 12" width="14" height="14" aria-hidden="true"><line x1="6" y1="2" x2="6" y2="10" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /><line x1="2" y1="6" x2="10" y2="6" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>
            <input
              type="text"
              placeholder={t.newReminder}
              spellcheck="false"
              autocomplete="off"
              bind:value={newReminder}
              oninput={() => (reminderError = "")}
              onkeydown={addReminder}
            />
          </div>
        {/if}
      </section>
    {/key}
  {/if}
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

  /* Same look as the pill's CSS-only glass (native blur was abandoned, see
     CLAUDE.md), but solid: this window is for reading, and anything showing
     through (WebView2 transparency renders lighter than the alpha suggests)
     hurts legibility. */
  .sheet {
    position: relative;
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
    transition: background 0.15s ease, transform 0.1s ease;
  }

  .close:hover {
    background: rgba(255, 255, 255, 0.2);
  }

  .close:active {
    transform: scale(0.9);
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 6px 18px 20px;
  }

  .scroll::-webkit-scrollbar {
    width: 6px;
  }

  .scroll::-webkit-scrollbar-thumb {
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.16);
  }

  h2 {
    margin: 20px 6px 8px;
    font-size: 0.78rem;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: rgba(245, 245, 247, 0.5);
  }

  .card {
    border-radius: 18px;
    background: rgba(255, 255, 255, 0.06);
    box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.05) inset;
  }

  .gauges {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 4px;
    padding: 16px 8px 14px;
  }

  .gauge {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    min-width: 0;
  }

  .ring {
    width: 76px;
    height: 76px;
    transform: rotate(-90deg);
  }

  .ring-track {
    fill: none;
    stroke: rgba(255, 255, 255, 0.1);
    stroke-width: 7;
  }

  .ring-value {
    fill: none;
    stroke-width: 7;
    stroke-linecap: round;
    stroke-dashoffset: var(--target);
    animation: ring-fill 1s cubic-bezier(0.16, 1, 0.3, 1) both;
  }

  @keyframes ring-fill {
    from {
      stroke-dashoffset: var(--full);
    }
    to {
      stroke-dashoffset: var(--target);
    }
  }

  .gauge-value {
    position: absolute;
    top: 0;
    height: 76px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .gauge-number {
    font-size: 1.15rem;
    font-weight: 700;
    letter-spacing: -0.02em;
  }

  .gauge-unit {
    font-size: 0.72rem;
    font-weight: 600;
    color: rgba(245, 245, 247, 0.6);
    margin-left: 1px;
  }

  .gauge-label {
    margin-top: 8px;
    font-size: 0.74rem;
    font-weight: 600;
    text-align: center;
  }

  .gauge-sub {
    margin-top: 1px;
    font-size: 0.66rem;
    color: rgba(245, 245, 247, 0.5);
    text-align: center;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
  }

  .list {
    padding: 4px 0;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 11px 14px;
  }

  .row + .row {
    box-shadow: 0 -1px 0 rgba(255, 255, 255, 0.06);
  }

  .row.column {
    flex-direction: column;
    align-items: stretch;
    gap: 7px;
  }

  .row-line {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 10px;
  }

  .row-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }

  .row-title {
    font-size: 0.88rem;
    font-weight: 600;
  }

  .row-detail {
    font-size: 0.78rem;
    line-height: 1.35;
    color: rgba(245, 245, 247, 0.6);
  }

  .badge {
    flex-shrink: 0;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 8px;
    color: #fff;
    background: var(--c);
    box-shadow: 0 2px 8px color-mix(in srgb, var(--c) 40%, transparent);
  }

  .bar {
    height: 6px;
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.1);
    overflow: hidden;
  }

  .bar span {
    display: block;
    height: 100%;
    border-radius: 3px;
    animation: bar-grow 0.9s cubic-bezier(0.16, 1, 0.3, 1) both;
    transform-origin: left;
  }

  @keyframes bar-grow {
    from {
      transform: scaleX(0);
    }
  }

  .chart {
    display: grid;
    grid-template-columns: repeat(24, 1fr);
    gap: 3px;
    padding: 16px 14px 10px;
  }

  .chart.week {
    grid-template-columns: repeat(7, 1fr);
    gap: 10px;
  }

  .chart-column {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
  }

  .chart-track {
    width: 100%;
    height: 88px;
    display: flex;
    align-items: flex-end;
  }

  .chart-bar {
    width: 100%;
    min-height: 2px;
    border-radius: 3px;
    background: #0a84ff;
    transform-origin: bottom;
    animation: chart-grow 0.8s cubic-bezier(0.16, 1, 0.3, 1) both;
  }

  @keyframes chart-grow {
    from {
      transform: scaleY(0);
    }
  }

  .chart-label {
    height: 12px;
    font-size: 0.66rem;
    color: rgba(245, 245, 247, 0.45);
    white-space: nowrap;
  }

  .footer {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    margin-top: 18px;
    font-size: 0.72rem;
    color: rgba(245, 245, 247, 0.45);
    text-align: center;
  }

  .pill-button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-top: 10px;
    border: none;
    border-radius: 999px;
    padding: 7px 14px;
    background: rgba(10, 132, 255, 0.18);
    color: #64b5ff;
    font: inherit;
    font-size: 0.8rem;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.15s ease;
  }

  .pill-button:hover:not(:disabled) {
    background: rgba(10, 132, 255, 0.28);
  }

  .pill-button:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .spinning {
    animation: spin 0.9s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .notification {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 12px 14px;
    margin-bottom: 8px;
  }

  .app-tag {
    font-size: 0.7rem;
    font-weight: 600;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: rgba(245, 245, 247, 0.55);
  }

  .body {
    user-select: text;
    -webkit-user-select: text;
  }

  .time {
    font-size: 1.35rem;
    font-weight: 600;
    letter-spacing: -0.02em;
    font-variant-numeric: tabular-nums;
  }

  .delete {
    flex-shrink: 0;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border: none;
    border-radius: 50%;
    background: rgba(255, 69, 58, 0.16);
    color: #ff6961;
    cursor: pointer;
    transition: background 0.15s ease;
  }

  .delete:hover {
    background: rgba(255, 69, 58, 0.3);
  }

  h2.danger {
    color: #ff6961;
  }

  .task {
    transition: opacity 0.3s ease;
  }

  .check {
    flex-shrink: 0;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: 1.8px solid rgba(245, 245, 247, 0.35);
    border-radius: 50%;
    background: transparent;
    color: transparent;
    cursor: pointer;
    transition:
      border-color 0.15s ease,
      background 0.2s ease,
      color 0.2s ease,
      transform 0.15s ease;
  }

  .check:hover {
    border-color: #0a84ff;
  }

  .check.red {
    border-color: rgba(255, 69, 58, 0.7);
  }

  .check:active {
    transform: scale(0.88);
  }

  .task.done .check {
    border-color: #0a84ff;
    background: #0a84ff;
    color: #fff;
  }

  .task.done .task-title {
    text-decoration: line-through;
    color: rgba(245, 245, 247, 0.4);
  }

  .task-title {
    transition: color 0.2s ease;
  }

  .late {
    color: #ff6961;
    font-weight: 600;
  }

  .add-task {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 0 18px 18px;
    padding: 12px 14px;
    border-radius: 16px;
    background: rgba(255, 255, 255, 0.07);
    color: #0a84ff;
    box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.05) inset;
  }

  .add-task input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: transparent;
    color: #f5f5f7;
    font: inherit;
    font-size: 0.88rem;
    user-select: text;
    -webkit-user-select: text;
  }

  .add-task input::placeholder {
    color: rgba(245, 245, 247, 0.38);
  }

  .add-error {
    margin: 0 22px 8px;
    font-size: 0.78rem;
    color: #ff6961;
  }

  .edit {
    flex-shrink: 0;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    padding: 0;
    border: none;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.07);
    color: rgba(245, 245, 247, 0.55);
    cursor: pointer;
    opacity: 0.6;
    transition:
      background 0.15s ease,
      color 0.15s ease,
      opacity 0.15s ease;
  }

  .task:hover .edit,
  .edit:focus-visible {
    opacity: 1;
  }

  .edit:hover {
    background: rgba(10, 132, 255, 0.2);
    color: #0a84ff;
  }

  .task.editing {
    flex-direction: column;
    align-items: stretch;
    gap: 8px;
  }

  .task.editing input {
    min-width: 0;
    border: none;
    outline: none;
    border-radius: 9px;
    padding: 7px 10px;
    background: rgba(255, 255, 255, 0.08);
    color: #f5f5f7;
    font: inherit;
    font-size: 0.85rem;
    color-scheme: dark;
    user-select: text;
    -webkit-user-select: text;
  }

  .task.editing input:focus {
    box-shadow: 0 0 0 1.5px #0a84ff inset;
  }

  .task.editing input:disabled {
    opacity: 0.4;
  }

  .edit-title {
    font-weight: 600;
  }

  .edit-when {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .edit-when input[type="date"] {
    flex: 1;
  }

  .edit-when input[type="time"] {
    width: 96px;
  }

  .clear-date {
    flex-shrink: 0;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border: none;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.1);
    color: rgba(245, 245, 247, 0.7);
    cursor: pointer;
  }

  .edit-actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }

  .text-button {
    border: none;
    border-radius: 9px;
    padding: 6px 12px;
    background: rgba(255, 255, 255, 0.08);
    color: #f5f5f7;
    font: inherit;
    font-size: 0.8rem;
    font-weight: 600;
    cursor: pointer;
  }

  .text-button.primary {
    background: #0a84ff;
  }

  .text-button:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 70px 24px;
    text-align: center;
  }

  .empty-glyph {
    width: 46px;
    height: 46px;
    color: rgba(245, 245, 247, 0.35);
    margin-bottom: 8px;
  }

  .empty-icon {
    display: grid;
    place-items: center;
    width: 46px;
    height: 46px;
    margin-bottom: 8px;
    border-radius: 50%;
    background: var(--c);
    font-size: 1.4rem;
    font-weight: 700;
  }

  .empty-title {
    font-size: 1rem;
    font-weight: 600;
  }

  .empty-hint {
    font-size: 0.8rem;
    color: rgba(245, 245, 247, 0.5);
    max-width: 280px;
  }
</style>
