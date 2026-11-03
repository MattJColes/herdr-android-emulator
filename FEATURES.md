# Features

What herdr-android-emulator does, in the order we build it. Each v1 item is done when its
"done when" line is true on macOS and Linux.

## Decisions

- **Frame source: the emulator's gRPC API.** The emulator runs with `-no-window -grpc <port>`
  and the pane reads `streamScreenshot` (~30fps). Input goes back through `sendTouch` and
  `sendKey`. `adb exec-out screencap` (~2fps) is the fallback for physical devices.
- **Platforms: macOS and Linux.** Linux needs KVM (`/dev/kvm`); Apple Silicon uses arm64
  system images.
- **Language: Rust**, following herdr-web-browser. Its terminal renderer (Kitty images,
  half-block fallback) and agent send code are copied, not shared, until both plugins work.
- **CLI name: `herdr-android`**, put on PATH (`~/.local/bin`) by a startup hook.

## v1

### 1. One-command SDK setup

`herdr-android install-sdk` installs cmdline-tools, platform-tools, the emulator and a
system image for this machine, accepts the licences and creates a default AVD with a sane
density. It finds an existing `ANDROID_HOME` and reuses it; otherwise it installs to the
standard place (`~/Library/Android/sdk`, `~/Android/Sdk`) so other Android tools share it. If JDK 17+ is missing it says
how to install it and stops.

Done when: a machine with only a JDK goes from nothing to a booted emulator with one command.

### 2. Emulator pane

**Android** in herdr's + menu opens a split beside the agent showing the live screen. The
emulator runs without its own window, so the pane is the only view.

- Click to tap, drag to swipe, scroll to scroll, type to send text and keys.
- Toolbar: back, home, recents, rotate, screenshot to agent, stop.
- Follows the herdr theme. Closing the pane leaves the emulator running.

Done when: you can sign in to an app in the pane without touching the emulator's own window.

### 3. Agent CLI

```sh
herdr-android boot [avd] --show        # start (or reuse) an emulator, show it in a pane
herdr-android devices                  # running emulators and devices
herdr-android install app.apk          # install or replace
herdr-android launch com.example.app   # start the app
herdr-android deeplink myapp://orders/42
herdr-android ui                       # on-screen elements: [3] Button "Sign in"
herdr-android tap 3                    # tap element 3 (or: tap 540 1200)
herdr-android type "hello" --submit
herdr-android swipe up
herdr-android key back
herdr-android screenshot /tmp/screen.png
```

`ui` comes from uiautomator, numbered like the browser's `snapshot` so agents use both
the same way.

Done when: an agent can install, launch and click through an app using only these commands.

### 4. Logs and events

A collector starts with the emulator and keeps logcat in a ring buffer under
`HERDR_PLUGIN_STATE_DIR`, so nothing is lost between commands.

```sh
herdr-android logcat --app com.example.app   # this app's lines (by PID), errors first
herdr-android crashes                        # FATAL EXCEPTION, ANR, native tombstones, with stacks
herdr-android events                         # boot, installs, app start/stop/death, rotation
```

Done when: after a crash, `crashes` shows the stack trace that caused it without the agent
re-running anything.

### 5. herdr integration

- `install-agents` teaches Claude Code, Codex, OpenCode and Pi to use `herdr-android`.
  Re-running it after an update replaces its own section.
- `uninstall` undoes everything the plugin added outside herdr: the CLI symlink and the
  agent instructions. It leaves the SDK and AVDs, which other tools share.
- Screenshot and log sends go through `herdr agent prompt`, using the browser's
  "which agent gets it" order.
- `pane.closed` / `pane.moved` events keep each pane tied to its emulator.
- Ctrl-click a `myapp://` or `intent://` link in any herdr terminal to open it on the device.
- Repo gets the `herdr-plugin` topic, which lists it on the marketplace, only after a
  manual test by the maintainer.

### 6. README with screenshots

Taken once 2–4 work: the pane beside an agent, the + menu entry, a crash in `crashes`.

## Later

- Crashes pushed to the agent as they happen (opt-in: it interrupts the agent).
- Snapshots: save, restore, quick boot.
- Physical devices over USB or wifi adb (no gRPC; screencap frames).
- Network capture (proxy, HAR); set location, battery and network conditions.
- One shared crate for the code both plugins copy.
