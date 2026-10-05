<p align="center">
  <img src="docs/logo.png" alt="Mimo" width="160" />
</p>

# Mimo

Mimo is a desktop companion for Windows that lives in a small floating pill at the top of the screen, inspired by the iOS Dynamic Island. It is designed to analyze what is happening on the computer and surface suggestions or automate small tasks for the user.

The project is in early development. Mimo already understands typed and spoken requests (French and English), learns from your activity and makes its first suggestions.

## Features

- **Floating pill** - borderless window at the top-center of the screen, responsive to screen size/DPI, auto-hides when idle, with a system tray icon and a right-click quick menu (local AI on/off with its download progress, settings, customize, close)
- **Character** - a tiny face in the pill (on by default) that reacts to what Mimo does: listens wide-eyed, looks around while thinking, smiles and hops when it worked, shakes its head when it didn't, tilts its head for a suggestion, trembles for an alarm, dozes off at night - and its eyes follow your pointer. Can be turned off in Customize to get the plain status dot back
- **Customize** - pick Mimo's color (12 presets or any color) for the pill, buttons and every window, optionally tint their glass with it, and show or hide the character; from the settings or the tray menu
- **Summon** - global shortcut (`F9` by default, any key or combination you like) for a typed request, or say "Hey Mimo" for a spoken one
- **Offline voice recognition** - [Vosk](https://alphacephei.com/vosk/) runs entirely on the machine; only the model for the selected language is loaded
- **Open apps and sites** - installed Start menu apps (desktop and Store), common websites and built-in Windows programs, by name ("open spotify", "ouvre youtube")
- **Search on platforms** - "mets squeezie sur youtube", "gotaga sur twitch", "damso feu de bois sur spotify", "ouvre spotify et affiche damso", "the latest video from inoxtag" - straight to the right page (YouTube, Twitch, Spotify app or web, Deezer, Netflix, Google Maps, Amazon, TikTok, X, Reddit…)
- **System controls** - "turn up the volume", "mets le son à 30", "mute", "baisse la luminosité" (built-in screens), "pause", "next song", "lock the PC", "mets le PC en veille"
- **Quick answers** - time, date and weather (for your area or a named city)
- **PC check-up** - CPU, memory, disk, temperature, battery and GPU readings with plain-language advice, shown in a dedicated panel
- **Notifications** - lists recent Windows notifications, and reads them aloud when asked by voice
- **Reminders and alarms** - "remind me in 10 minutes to...", "set an alarm for 7:30", or typed straight into the reminders panel ("call mom tomorrow at 6pm")
- **Tasks** - a simple to-do list with optional days and times, managed by voice or text, each task editable in place (title, day, time)
- **Screenshots and screen recordings** - "take a screenshot" (PNG in `Pictures\Mimo Capture`), "record the screen" / "stop recording" (MP4 in `Videos\Mimo Records`); the pill never shows up in them
- **Small talk** - say hi, ask how it's going, say thanks or good night: Mimo answers back
- **Help** - "I need help" asks whether it's an emergency (with the emergency numbers) or a question about Mimo (links to this repository); the pill's "?" button and the tray menu's "A problem?" lead here too
- **Your own commands** - in the settings, "My commands" lets you choose what a phrase does: a reply, a website, an installed app or another request
- **Translation** - translate typed text, or whatever you copy next, between French and English
- **Activity insights** - learns which apps you use and when (stored only on your PC, 30 days); ask for your screen time or a summary of your day
- **Look back** - "what was I doing yesterday around 3 pm?", "qu'est-ce que j'ai fait ce matin ?" (the apps and window titles of that moment), and "reopen what I had open" / "rouvre ce que j'avais ouvert hier" to relaunch those apps (apps only, not documents or tabs)
- **Suggestions** - offers on its own to open the apps you usually start around this time, to close an app that's slowing the PC down, to empty a full recycle bin, to take a break or to go to bed - never over a fullscreen game or video (except a low battery warning), never stealing focus
- **Low battery** - reminds you to plug in the charger at 20 %, more urgently at 10 %
- **Local AI (optional, off by default)** - turn it on in the settings and Mimo downloads, once and in the background (progress bar, resumable), a small language model ([Qwen3 4B Instruct](https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507), Apache 2.0, ~2.5 GB) and the [llama.cpp](https://github.com/ggml-org/llama.cpp) engine (MIT). It then runs entirely on your PC - free, no account, nothing sent online. It understands requests worded any way ("I can't hear anything" → turns the volume up; it only ever picks one of Mimo's own actions), answers free questions ("what is photosynthesis?"), summarizes, fixes or rephrases the text you copied, and words your day's summary naturally. The engine starts when needed, uses your graphics card if it can (Vulkan), runs at low priority and stops after 10 minutes unused
- **Settings** - language (English / French), launch at Windows startup, summon shortcut, "Hey Mimo" on/off, activity analysis and suggestions on/off (separately), sounds, local AI (with its download progress, and a button to delete its files), customize, your own commands, erase all local data; the version is shown at the bottom

## Privacy

Mimo has no telemetry and keeps its data (settings, reminders, tasks, your commands, activity history) on the PC; screenshots and recordings are saved to your own Pictures and Videos folders. Voice recognition is fully offline. The only network requests are:

- opening the URLs you ask for
- weather: [Open-Meteo](https://open-meteo.com/) for the forecast and city lookup, and [ipwho.is](https://ipwho.is/) to approximate your location from your IP when no city is given
- translation: the text to translate is sent to Google Translate
- local AI, only if you turn it on: a one-time download of the engine from GitHub and of the model from Hugging Face (after that, the AI works offline)

## Architecture

The project is a Cargo workspace, split so the core logic stays independent of the UI:

- `crates/core` (`mimo-core`) - engine, settings and all the logic: request parsing (FR/EN), platform searches, system controls, looking back at activity, small talk, your own commands, app matching, reminders, tasks, check-up advice, suggestions. No Tauri dependency, unit-testable on its own.
- `crates/commands` (`mimo-commands`) - thin bridge exposing `mimo-core` to the frontend as Tauri commands.
- `apps/desktop` - the Tauri shell: Svelte/TypeScript frontend (pill, tray menu, panel, commands window), window management, and OS integration (voice, sounds, volume/brightness/media keys, installed apps, notifications, system readings, screen capture, persistence).

This separation means the core engine can evolve, be tested, and eventually be reused without being coupled to how the UI is built.

## Tech stack

- [Tauri 2](https://tauri.app/) (Rust) for the native shell - small footprint compared to an Electron-based app
- Svelte 5 + SvelteKit (static/SPA mode) + TypeScript for the frontend
- Windows only for now; cross-platform support is not a current goal

## Development

Requirements: Rust (stable), Node.js, npm.

```bash
cd apps/desktop
npm install
npm run tauri dev
```

`npm run tauri dev` first runs `npm run setup:voice`, which downloads the Vosk runtime and the French/English models into `src-tauri/resources/vosk/` (once; it is skipped when they are already there). The app still starts without them, with voice disabled.

Build the installer (`target/release/bundle/nsis/Mimo_<version>_x64-setup.exe`, per-user, voice models included):

```bash
cd apps/desktop
npm run tauri build
```

Run the tests (from the repository root):

```bash
cargo test --workspace
```

## License

MIT, see [LICENSE](LICENSE).
