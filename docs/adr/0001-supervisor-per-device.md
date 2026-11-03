# A Supervisor process per Device

Each Device is owned by a detached Supervisor that `boot` starts. The Supervisor runs the
emulator, keeps logcat and events in a ring buffer under the plugin state dir, and serves
Views and CLI commands over a Unix socket. herdr has no plugin daemons (startup hooks are
one-shot), and the promise that a crash is still there when the agent asks later can't be
kept by reading `adb logcat -d` on demand: logcat's own buffer rolls over.

herdr-ios-simulator has no Supervisor: the iOS unified log is a persistent store, so
`log show --last` on demand loses nothing. This is a platform difference, not drift.

## Considered Options

- **No Supervisor, `adb logcat -d` per command.** Simpler, but old crashes are lost and
  every View opens its own gRPC stream.
- **Supervisor per user.** One process for all Devices; a bug or crash in it takes every
  Device's logs down at once.
