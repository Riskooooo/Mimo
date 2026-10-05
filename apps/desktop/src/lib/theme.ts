// Mimo's main color (setting `accent_color`, "Personnaliser" window), applied
// to every window as CSS variables on :root:
//   --accent        the color itself
//   --accent-rgb    "r, g, b", for rgba(var(--accent-rgb), a)
//   --on-accent     black or white, whichever reads on the color
//   --accent-text   a lighter shade, for text on a dark background
//   --tint          how much the glass is tinted with it (0 when off)
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export const DEFAULT_ACCENT = "#0a84ff";
const TINT = 0.22;

export type Appearance = { accent_color: string; tinted_glass: boolean };

// Apple's system colors, which suit the dark glass.
export const PRESETS: { color: string; fr: string; en: string }[] = [
  { color: "#0a84ff", fr: "Bleu", en: "Blue" },
  { color: "#5e5ce6", fr: "Indigo", en: "Indigo" },
  { color: "#bf5af2", fr: "Violet", en: "Purple" },
  { color: "#ff375f", fr: "Rose", en: "Pink" },
  { color: "#ff453a", fr: "Rouge", en: "Red" },
  { color: "#ff9f0a", fr: "Orange", en: "Orange" },
  { color: "#ffd60a", fr: "Jaune", en: "Yellow" },
  { color: "#32d74b", fr: "Vert", en: "Green" },
  { color: "#66d4cf", fr: "Menthe", en: "Mint" },
  { color: "#64d2ff", fr: "Cyan", en: "Cyan" },
  { color: "#ac8e68", fr: "Sable", en: "Sand" },
  { color: "#98989d", fr: "Graphite", en: "Graphite" },
];

function rgb(hex: string): [number, number, number] {
  const value = /^#([0-9a-f]{6})$/i.exec(hex)?.[1] ?? DEFAULT_ACCENT.slice(1);
  return [0, 2, 4].map((i) => parseInt(value.slice(i, i + 2), 16)) as [number, number, number];
}

/** Relative luminance (WCAG), 0 = black, 1 = white. */
function luminance([r, g, b]: [number, number, number]): number {
  const channel = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

export function applyTheme(appearance: Appearance) {
  const color = /^#[0-9a-f]{6}$/i.test(appearance.accent_color) ? appearance.accent_color : DEFAULT_ACCENT;
  const [r, g, b] = rgb(color);
  const style = document.documentElement.style;
  style.setProperty("--accent", color);
  style.setProperty("--accent-rgb", `${r}, ${g}, ${b}`);
  style.setProperty("--on-accent", luminance([r, g, b]) > 0.45 ? "#1c1c1e" : "#ffffff");
  style.setProperty("--accent-text", `color-mix(in srgb, ${color} 62%, white)`);
  style.setProperty("--tint", appearance.tinted_glass ? String(TINT) : "0");
}

/** Applies the saved theme now and whenever it changes; returns the cleanup. */
export function watchTheme(): () => void {
  void invoke<Appearance>("get_settings").then(applyTheme);
  const unlisten = listen<Appearance>("mimo://settings-changed", (event) => applyTheme(event.payload));
  return () => void unlisten.then((fn) => fn());
}
