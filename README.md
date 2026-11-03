# herdr-android-emulator

An Android emulator built into [herdr](https://herdr.dev). The emulator's screen opens in a pane next to your coding agent, so you and your agent use the same device. The agent can install and click through your app, and read its logs, crashes and events, while you watch.

> **Status: early development.** Nothing below works yet. [FEATURES.md](FEATURES.md) lists what's being built and in what order. Screenshots come once the pane works.

## What it will do

- **Opens from herdr's + menu.** Pick **Android** and the emulator's screen opens beside your agent. Click to tap, drag to swipe, type to type. There's no separate emulator window.
- **Set up in one command.** `herdr-android install-sdk` installs the Android SDK, a system image and a ready-to-boot device. No Android Studio.
- **Agents can drive it.** Install an APK, launch the app, list what's on screen, tap, type and take screenshots.
- **Agents see what happened.** Your app's logcat, crashes with stack traces, and device events, kept from the moment the emulator boots.

## Requirements

- herdr 0.9 or later, on macOS or Linux
- Rust (herdr builds the plugin with cargo when it installs it)
- JDK 17 or later (for the Android SDK tools)
- Linux: KVM (`/dev/kvm`) for a usable emulator speed

## Install

```sh
herdr plugin install MattJColes/herdr-android-emulator
```

## Update

```sh
herdr plugin update herdr-android-emulator
herdr-android install-agents    # refresh what your agents know about new commands
```

The update rebuilds the plugin. A running emulator keeps running; close and reopen the Android pane to pick up the new version.

## Uninstall

```sh
herdr-android uninstall          # removes the CLI from ~/.local/bin and its section from your agents' instructions
herdr plugin uninstall herdr-android-emulator
```

Run them in that order: `herdr-android` is gone after the second one. Your settings stay in herdr's plugin config folder (`herdr plugin config-dir herdr-android-emulator` shows where); delete it to start fresh next time.

The Android SDK and your virtual devices stay, because Android Studio, Flutter and React Native use the same ones. To remove them too:

```sh
rm -rf ~/Library/Android/sdk ~/.android/avd   # macOS
rm -rf ~/Android/Sdk ~/.android/avd           # Linux
```

## Development

```sh
git clone git@github.com:MattJColes/herdr-android-emulator.git
cd herdr-android-emulator
git config core.hooksPath .githooks   # fmt, clippy and tests before each commit
herdr plugin link .                    # run your checkout inside herdr
cargo build --release
```

Commits follow [Conventional Commits](https://www.conventionalcommits.org). Release Please turns them into the changelog and version bumps.

## License

[MIT](LICENSE)
