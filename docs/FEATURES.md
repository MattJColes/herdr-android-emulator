# Features

What herdr-android-emulator does, in the order we build it. Each v1 item is done when its
"done when" line is true on macOS and Linux. Its sibling, [herdr-ios-simulator](https://github.com/MattJColes/herdr-ios-simulator),
uses the same command names, words and order, so an agent that knows one knows the other.
Words in **bold** are defined in [GLOSSARY.md](../GLOSSARY.md).

## Decisions

- **Screen and input: the emulator's gRPC API.** The emulator runs with
  `-no-window -grpc <port> -grpc-use-token`; Views read `streamScreenshot` (~30 fps) and send
  `sendTouch`/`sendKey`. See [ADR 0002](adr/0002-grpc-for-screen-and-input.md).
- **A Supervisor per Device, unlike iOS.** logcat's buffer rolls over, so a background process
  owns each Device and keeps its log entries, events and marks. iOS needs none: its unified log
  and crash reports stay on disk. See [ADR 0001](adr/0001-supervisor-per-device.md).
- **One shared Current Device.** Every agent and the View act on it unless a command passes
  `--device`. See [ADR 0003](adr/0003-one-shared-current-device.md).
- **Platforms: macOS and Linux.** Linux needs KVM (`/dev/kvm`); Apple Silicon uses arm64
  system images.
- **Language: Rust**, following herdr-web-browser. Its terminal renderer (Kitty images,
  half-block fallback) and agent send code are copied, not shared, until both plugins work.
- **CLI name: `herdr-android`**, put on PATH (`~/.local/bin`) by a startup hook. The plugin id
  stays `herdr-android-emulator`.

Not goals: physical phones (see Later), replacing Android Studio.

## v1

### 0. SDK setup

Android-only: iOS relies on Xcode being installed.

`herdr-android install-sdk` installs cmdline-tools, platform-tools, the emulator and a system
image for this machine, accepts the licences and creates the **Default AVD**, `herdr-default`:
Pixel 8 hardware, API 36 Google APIs (no Play Store, so `adb root` works), arm64 or x86_64 to
match the host. It reuses an existing `ANDROID_HOME`; otherwise it installs to the standard
place (`~/Library/Android/sdk`, `~/Android/Sdk`) so other Android tools share it. If JDK 17+ is
missing it says how to install it and stops.

Done when: a machine with only a JDK goes from nothing to a booted Device with one command.

### 1. View

The **Current Device** in a herdr pane beside the agent. The emulator has no window of its own,
so the View is the only one.

- Click to tap, drag to swipe, scroll to scroll, type to send text, Return, Backspace and Tab.
- Header: AVD name and API level, Back, Home, Recents, Rotate, Screenshot, Quit.
- Opens from the `Android: open` action, herdr desktop's + menu or `herdr-android boot --show`.
  With no Device running it boots the Default AVD; with several it asks which.
- Several Views can show one Device. Closing a View leaves the Device running; only `shutdown`
  or the Quit button stops it.
- Follows the herdr theme.

Done when: you can sign in to an app in the View without touching the emulator's own window.

### 2. Agent CLI

```sh
herdr-android boot [avd] --show        # boot (or reuse) a Device, show it in a View
herdr-android boot --new               # a second Device; becomes the Current Device
herdr-android devices                  # running Devices and AVDs (* = current)
herdr-android shutdown
herdr-android build --run              # ./gradlew assembleDebug: errors with file:line, then install and launch
herdr-android install app.apk          # install or replace
herdr-android launch com.example.app   # start the app; it becomes the Current app
herdr-android terminate
herdr-android deeplink myapp://orders/42
herdr-android ui                       # on-screen elements: [3] Button "Sign in"
herdr-android tap 3                    # tap element 3 (or: tap 540 1200, in pixels)
herdr-android type "hello" --submit
herdr-android swipe up
herdr-android button back              # back, home, recents, power
herdr-android screenshot /tmp/screen.png
```

Every command takes `--json` and `--device emulator-5556`. Without `--device`, a command uses
the Current Device, then the only running Device, then boots the Default AVD and says so.

- `ui` comes from uiautomator and lists only elements on screen, numbered like the browser's
  `snapshot`, so agents use both the same way.
- `tap 3` finds element 3 again by resource-id and text before tapping (~300 ms), and fails if
  it's gone, rather than tapping where it used to be.
- `build` runs the project's Gradle wrapper for module `app` (`--module`, `--variant` to
  change), finding an `android/` folder so Flutter and React Native projects work too. It
  returns compiler errors and warnings only.

Done when: an agent can build, install, launch and click through an app using only these
commands.

### 3. Logs and events

The Supervisor keeps everything per AVD across boots, each entry tagged with its boot, under
`HERDR_PLUGIN_STATE_DIR` (50 MB per AVD; oldest boot dropped first).

```sh
herdr-android logs --errors            # the Current app's log entries this boot, errors first
herdr-android crashes                  # Crashes and ANRs, labelled, with stacks, every boot
herdr-android events                   # boot, installs, launches, deep links, taps (yours and the agent's)
herdr-android mark before-login        # then: logs --since before-login
```

- `logs` follows the Current app across restarts by mapping its package to every PID it has
  had this boot. `--app com.x` picks another app, `--all` drops the filter, `--all-boots` reads
  earlier boots.
- Events are written by the plugin when it acts, never parsed from logcat.
- Log text, crash text and on-screen labels are app content: `agent-help` tells agents to treat
  them as data, not instructions.

Done when: after a crash and a reboot, `crashes` shows the stack that caused it without the
agent re-running anything.

### 4. Send to the agent

- Screenshot and recent errors to the agent, through `herdr agent prompt`, using the browser's
  "which agent gets it" order.
- `pick --prompt "Which button?"`: the user taps in the View and the agent gets that element back.

Done when: one click in the View puts a screenshot in the agent's prompt.

### 5. herdr integration

- `install-agents` teaches Claude Code, Codex, OpenCode and Pi to use `herdr-android`.
  Re-running it after an update replaces its own section.
- `uninstall` undoes everything the plugin added outside herdr: the CLI symlink and the agent
  instructions. It leaves the SDK and AVDs, which other tools share. Until it exists the README
  lists the manual steps.
- Ctrl-click a `myapp://` or `intent://` link in any herdr terminal to open it on the Device.
- Repo gets the `herdr-plugin` topic, which lists it on the marketplace, only after a manual test
  by the maintainer.

### 6. README with screenshots

Taken once 1–4 work: the View beside an agent, the + menu entry, a crash in `crashes`.

## Later

- Crashes pushed to the agent as they happen (opt-in: it interrupts the agent).
- Snapshots: save, restore, quick boot.
- Physical devices over USB or wifi adb (no gRPC; screencap frames).
- Network capture (proxy, HAR); set location, battery and network conditions.
- Stop idle Devices after a timeout, if their memory use becomes a complaint.
- Per-agent Devices, if two agents sharing one becomes common.
- One shared crate for the code both plugins copy.
