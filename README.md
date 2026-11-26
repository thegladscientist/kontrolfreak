# Kontrolfreak

Modular, low-latency desktop studio for routing control signals, hardware peripherals, video streams, and depth sensors across creative apps.

## Highlights

- **Core Signal Router (M1):** Lock-free `kf-core` routing engine, OSC, MIDI (including virtual ports), WebSockets, raw UDP, modifier chains (range, curve, deadzone, hysteresis, smoothing, expressions), collapsible racks with macros, Offline Ghost device auto-rebind, and 3-pane hybrid UI with Calm Mode.
- **Pure-Rust RTPMIDI & Dynamic NDI Video (M2):** Full RFC 6295 / AppleMIDI (dual UDP `N`/`N+1`, 3-way clock sync, recovery journal, mDNS-SD discovery) and dynamic NDI v6 runtime loading with arbitrary resolution scaling, crop/pan/zoom/letterbox, ~150 ms debounced resize, built-in Test Patterns, and live signal modulation.
- **Generic HID Auto-Learn & RGB Camera CV (M3):** Hot-plug USB/Bluetooth HID discovery, 4-step noise-masked HID Learn Wizard (auto-classifies buttons, faders, joysticks, rotary encoders), reusable device presets, and lightweight CPU RGB camera CV extractors (luma, motion energy, color, blob centroid, zones).
- **Crash-Isolated Depth Sidecars & `.kfreak` Bundles (M4):** Separate-process depth sensor sidecars (`kf-sensor-depth`) with zero-copy shared-memory ring buffers, automatic watchdog restart, 3D ROI extractors + NDI depth matte, portable `.kfreak` zip project bundles, and live latency/sidecar Diagnostics & Profiler drawer.

---

## Prebuilt Desktop Installers (Windows & macOS)

You can download ready-to-install desktop builds (with the bundled `kf-sensor-depth` sidecar) straight from GitHub — no local Rust/C++ toolchain required:

- **One-click build via GitHub Actions:** Open the **Actions** tab → select **Build & Release Desktop Installers** → click **Run workflow** (optionally check *Also create/update a draft GitHub Release*). When the run finishes, download the artifact for your platform:
  - `kontrolfreak-windows-x64` — Windows `.msi` and NSIS `-setup.exe` installers
  - `kontrolfreak-macos-apple-silicon` — macOS Apple Silicon (`.dmg`, M1/M2/M3/M4)
  - `kontrolfreak-macos-intel` — macOS Intel (`.dmg`, x86_64)
- **Tagged GitHub Releases:** Pushing a version tag (for example `git tag v0.1.0 && git push origin v0.1.0`) automatically builds all Windows and macOS installers, computes `SHA256SUMS.txt`, and publishes a draft release under **Releases**.

### Unsigned Build Notice (macOS Gatekeeper & Windows SmartScreen)
Default workflow builds are unsigned unless optional Apple Developer ID / Windows signing secrets are configured in repository settings:
- **macOS:** On first launch, right-click **Kontrolfreak.app** → **Open** → **Open** (or run `xattr -cr /Applications/Kontrolfreak.app` if Gatekeeper reports the app is damaged/unverified).
- **Windows:** If SmartScreen displays *"Windows protected your PC"*, click **More info** → **Run anyway**.

---

## Quick Start (Build from Source — Windows & macOS)

### Prerequisites
- **Node.js 20+** and `npm`
- **Rust stable toolchain** (`rustup`)
- **Platform build tools (for the native Tauri desktop app):**
  - **Windows:** [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) and WebView2 Runtime (pre-installed on Windows 10/11).
  - **macOS:** Xcode Command Line Tools (`xcode-select --install`).
  - **Linux:** `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libasound2-dev`, `librsvg2-dev`, `libayatana-appindicator3-dev`, `libssl-dev`, `libudev-dev`, `pkg-config`.

### 1. Install dependencies
```bash
npm install
```

### 2. Run the native desktop app (Tauri v2 + Rust backend)
```bash
npm run tauri dev
```

To build a local release installer with the `kf-sensor-depth` sidecar bundled:
```bash
npm run tauri:build
```

### 3. Or preview in a standalone browser (loopback mock mode)
No Rust build or hardware required — includes synthetic NDI Test Patterns, mock RTPMIDI peers, simulated HID Learn devices, synthetic RGB camera CV, and synthetic Depth sensors:
```bash
npm run dev
```
Then open `http://localhost:5173` in your browser.

---

## Optional Runtime Notes

### NDI Runtime v6 (Optional)
Kontrolfreak loads the NDI v6 shared library dynamically at runtime (`Processing.NDI.Lib.x64.dll` on Windows, `libndi.dylib` on macOS, `libndi.so` on Linux).
- If **NDI Runtime** is not installed, Kontrolfreak starts cleanly and provides built-in animated **Test Pattern** NDI sources (SMPTE Bars, Motion Grid, Animated Gradient) so resizing, cropping, FPS throttling, and live signal modulation work immediately.
- To connect real NDI sources/receivers, install [NDI Tools / NDI Runtime](https://ndi.video/tools/) and restart Kontrolfreak.

### Virtual MIDI Ports (`loopMIDI` on Windows)
- **macOS:** Virtual MIDI ports are created natively via CoreMIDI — no extra driver needed.
- **Windows:** Windows does not include a built-in user-space virtual MIDI driver. Install [Tobias Erichsen's loopMIDI](https://www.tobias-erichsen.de/software/loopmidi.html), create a virtual port (e.g. `Kontrolfreak Bus 1`), and Kontrolfreak's **Devices** sidebar will detect it automatically and guide setup.

---

## Running Tests & Checks

### Rust Workspace
```bash
# Strict zero-warning check across all crates + Tauri shell
RUSTFLAGS="-D warnings" cargo check --workspace

# Run all M1-M4 Rust unit & integration tests
cargo test --workspace

# Optional feature checks (native USB HID + wgpu GPU video scaler)
cargo check -p kf-hardware --features native-hid
cargo test -p kf-video --features gpu
```

### Frontend (TypeScript, Vitest, Vite)
```bash
npx tsc --noEmit
npm test
npm run build
```

---

## Workspace Layout

| Path | Description |
| :--- | :--- |
| `crates/kf-core` | Lock-free routing engine, modifiers, racks/macros, presets, telemetry |
| `crates/kf-protocols` | OSC, MIDI, WebSockets, UDP, and pure-Rust RFC 6295 RTPMIDI + mDNS-SD |
| `crates/kf-video` | Dynamic NDI v6 loader, Test Patterns, SIMD/wgpu scaler, live modulation |
| `crates/kf-hardware` | HID hot-plug, 2-tier descriptor & noise-masked Learn engine, RGB CV |
| `crates/kf-sidecar-ipc` | Sidecar supervisor/watchdog, SHM ring buffer, `.kfreak` zip bundles, profiler |
| `crates/kf-sensor-depth` | Crash-isolated depth sensor sidecar binary (RealSense/Orbbec/Kinect/Synthetic) |
| `src-tauri` | Tauri v2 desktop host & IPC commands (`commands.rs`, `commands_m2..m4.rs`) |
| `src/` | React + TypeScript + Tailwind hybrid UI & modular M2–M4 plugins (`src/plugins/`) |
