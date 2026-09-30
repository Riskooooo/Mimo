# Mimo

Mimo is a desktop companion for Windows that lives in a small floating pill at the top of the screen, inspired by the iOS Dynamic Island. It is designed to analyze what is happening on the computer and surface suggestions or automate small tasks for the user.

The project is in early development. The current focus is the desktop shell and interaction model; the actual system-analysis and automation features are not built yet.

## Status

- Floating, borderless "pill" window, positioned top-center and responsive to screen size/DPI
- Animated reveal sequence (collapsed dot -> loading -> ready) with a status indicator
- Minimize to system tray, with a right-click quick menu (Settings / Close)
- Settings drawer: launch at Windows startup, erase local data
- No system analysis, automation, or data collection yet - this is still UI/shell scaffolding

## Architecture

The project is a Cargo workspace, split so the core logic stays independent of the UI:

- `crates/core` (`mimo-core`) - engine, settings, and (later) system-analysis/automation logic. No Tauri dependency, unit-testable on its own.
- `crates/commands` (`mimo-commands`) - thin bridge exposing `mimo-core` to the frontend as Tauri commands.
- `apps/desktop` - the Tauri shell: Svelte/TypeScript frontend, window management, tray, OS integration (autostart, local settings persistence).

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

Run the core crate's tests:

```bash
cargo test -p mimo-core
```

## License

MIT, see [LICENSE](LICENSE).
