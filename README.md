# AI Hub

A multi-SSB with a sidebar for AI chats: Claude, ChatGPT, Gemini, Grok, DeepSeek, Perplexity and any
service you add yourself. Every tab lives in its own webview, so switching is instant and loses no
state — the open chat, the text you typed and a running generation all survive.

Tauri 2 (Rust) + vanilla JS, no bundler. Uses the system WebView2 on Windows and WebKitGTK on Linux.

## Features

- Sidebar of service icons; the list is stored in `tabs.json`
- Add a tab from presets or by your own URL, icons are fetched as favicons
- Auto-unload of inactive tabs after a configurable timeout, manual pinning with a right click
- Restores the last tab and its URL on startup
- Three themes (dark, light, "original"); the theme is passed down to the sites themselves
- UI in 7 languages: Russian, Ukrainian, Belarusian, English, Polish, German, Lithuanian

## Development

Requires Rust (stable) and Node 18+. The frontend is not built — it is plain static files in `src/`.

```
npm install
npm run tauri dev
```

## Building installers

### Windows (NSIS)
```
npm run tauri build
```
The installer lands in `src-tauri/target/release/bundle/nsis/AI Hub_0.1.0_x64-setup.exe`.

It asks for the language (Russian, Ukrainian, English, Polish, German) and the install mode (all
users or the current user only), lets you pick the install directory, and creates Start menu and
desktop shortcuts. NSIS itself is downloaded by the Tauri CLI on the first build.

### Arch Linux

Pack the tree on the machine you develop on:

```
tar czf ai-hub.tar.gz --exclude=node_modules --exclude=target --exclude=.git src src-tauri packaging
```

Then, on the target machine, two steps are enough:

```
tar xzf ai-hub.tar.gz
cd packaging && makepkg -si
```

`makepkg -si` pulls the missing dependencies through pacman (`webkit2gtk-4.1`, `gtk3`, `rust`),
builds the package and installs it: the binary at `/usr/bin/ai-hub`, a launcher in the application
menu and icons in `hicolor`. Uninstall the usual way — `sudo pacman -R ai-hub`.

Node and npm are not needed there: the frontend is plain static files and cargo builds the binary.

An AppImage is the alternative if you would rather not build a package:
`npm run tauri build -- --bundles appimage` (must be built on Linux), the file appears in
`src-tauri/target/release/bundle/appimage/`.

## Known limitations

- Wayland: child webviews can only be positioned through X11, so the app forces XWayland
  (`GDK_BACKEND=x11`).
- Auto-unload is not paused while a response is streaming: the app cannot see network activity
  inside someone else's page. Pin the tab instead.
- Split view (several tabs side by side) is not implemented yet.
