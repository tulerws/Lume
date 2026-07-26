# Lume

Lume is a subtle, local desktop overlay for monitoring AI coding agents on Windows and Linux. Its compact capsule shows when agents are running, waiting for input, requesting permission, completing work, or failing. Expand it whenever you need the full session hub.

[Download the latest release](https://github.com/tulerws/Lume/releases/latest)

## Features

- **Unified agent monitoring** for Codex, Claude, and Gemini across CLIs, VS Code, and supported Chromium browsers.
- **Real-time session status** with a quiet animated mascot, active-agent counter, optional sounds, and system tray access.
- **Permission handling** with a clear description of the requested action and direct approval or denial whenever the source supports it.
- **Per-session chat hub** showing your prompts, agent responses, observable activity, commands, tools, and final results.
- **Changed-file tracking** grouped by prompt, plus a consolidated Changes tab for the entire session.
- **Floating Whiteboard terminals** that can be moved, resized, docked horizontally or vertically, and restored through saved layouts.
- **Prompt continuation** for compatible Codex sessions and connected web chats without returning to the original application.
- **Session launcher** for opening or resuming agents in your usual terminal or the VS Code integrated terminal.
- **Command palette and global shortcuts** for opening Lume, launching sessions, navigating the app, and opening Whiteboard terminals.
- **Project profiles** for launch destination, monitor, overlay position, permission behavior, Whiteboard layout, and preferred agents.
- **Local result notes** for explicitly saving useful final responses, reported files, and validation checks.
- **Extensible detection** through declarative JSON manifests that never load or execute third-party code.
- **Local-first privacy** with sanitized history, in-memory session content, and services bound exclusively to localhost.
- **Automatic updates**, autostart, configurable monitor placement, light/dark themes, and fullscreen-aware overlay behavior.

## Supported sources

| Source | Detection | Direct permission actions |
| --- | --- | --- |
| Claude CLI | Processes and hooks | Yes |
| External Codex CLI/VS Code sessions | Processes, rollouts, and hooks | Monitoring only |
| Codex sessions opened by Lume | Local App Server | Yes |
| Gemini CLI | Processes and hooks | Monitoring only |
| ChatGPT, Claude, and Gemini web | Chromium Companion | Opens the matching tab |

Lume only displays actions supported by the current session. It never simulates an approval that the source integration cannot perform. The session hub shows only observable information exposed by the agent, App Server, or hooks. Private model reasoning is never available.

## Install

The [latest GitHub release](https://github.com/tulerws/Lume/releases/latest) provides:

- Windows NSIS installer (`.exe`)
- Debian/Ubuntu package (`.deb`)
- Fedora/RHEL package (`.rpm`)
- Portable Linux AppImage
- Android application (`Lume-Mobile.apk`)

Lume checks for updates automatically and lets you install them from **Settings → About**.
Lume Mobile checks the signed Android release automatically and verifies its SHA-256 hash before opening the system installer. The PWA refreshes its cached application files automatically. iOS updates remain managed by TestFlight or the App Store.

## Run in development

Requirements: Node.js 22+, stable Rust, and the Tauri system dependencies.

On Pop!_OS or Ubuntu:

```bash
sudo apt-get update
sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev libgtk-layer-shell0 build-essential curl wget file libssl-dev libayatana-appindicator3-dev librsvg2-dev libdbus-1-dev pkg-config
```

Then run:

```bash
npm install
npm run check
npm run tauri dev
```

Lume opens on the primary monitor and adds an icon to the system tray. Use **Settings** to connect installed agents, configure VS Code, install the browser Companion, and customize shortcuts.

## Connect agent sources

After connecting Codex for the first time, run `/hooks` inside Codex and trust the **Lume** hook. Codex requires this confirmation for new or modified local hooks.

To install the browser Companion:

1. Open **Settings → Browsers → Open folder** in Lume.
2. Visit `chrome://extensions` or the equivalent page in Edge or Brave.
3. Enable developer mode.
4. Load the Companion folder as an unpacked extension.

The Companion sends only the agent type, state, sanitized title, source, and a local hash of the path. Prompts submitted through Lume travel only over the local connection to the selected tab and are not stored in Lume history.

## Linux display support

On compatible Wayland compositors, Lume uses Layer Shell for monitor-aware placement. The `.deb` package installs `libgtk-layer-shell0`; AppImage users should install that package separately when native Layer Shell behavior is desired.

Fedora Workstation with GNOME Wayland automatically uses the XWayland fallback because GNOME does not expose Layer Shell. This keeps dragging and saved overlay positions functional. Set `LUME_FORCE_NATIVE_WAYLAND=1` to test the native backend explicitly.

Global shortcuts are registered directly on Windows and integrated with the desktop shortcut systems used by COSMIC and GNOME.

## Keyboard shortcuts

Default shortcuts:

| Action | Shortcut |
| --- | --- |
| Open Lume | `Ctrl+Alt+Shift+L` |
| Open command palette | `Ctrl+Shift+Space` |
| Start a new session | `Ctrl+Alt+Shift+N` |
| Open the Whiteboard | `Ctrl+Alt+Shift+B` |

All shortcuts can be changed from **Settings → Keyboard shortcuts**. `Ctrl+Shift+P` also opens the command palette while the Lume window is focused.

## Build installers

```bash
npm run tauri build
```

Linux bundles are written to `src-tauri/target/release/bundle`. The **Installers** GitHub Actions workflow builds `.deb`, `.rpm`, AppImage, and Windows NSIS packages, creates the GitHub Release, signs updater artifacts, and publishes the `latest.json` manifest.

Before publishing a release, configure `TAURI_SIGNING_PRIVATE_KEY` in **Settings → Secrets and variables → Actions**. The public key belongs in the application configuration; the private key must never be committed. Update the version in `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`, then push a `v*` tag.

Android releases also require `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, and `ANDROID_KEY_PASSWORD` as GitHub Actions secrets. Keep a secure backup of that keystore: Android only accepts automatic updates signed by the same certificate as the installed application.

## External detectors

Use **Settings → External detectors** to install a JSON manifest for another CLI. See [`docs/external-plugin.example.json`](docs/external-plugin.example.json) for the format.

Manifests only declare process names and matching tokens. They do not load libraries or execute commands inside Lume. Detector changes take effect on the next scan without restarting the application.

## Privacy

Everything stays on the machine. Lume services listen only on `127.0.0.1:43119`, `127.0.0.1:43120`, `127.0.0.1:43130`, and `127.0.0.1:43131`.

Session messages and final responses remain in memory. SQLite stores preferences, sanitized history summaries, and only the responses the user explicitly chooses to save as notes.

Read more in [Product](docs/PRODUCT.md) and [Privacy](docs/PRIVACY.md).
