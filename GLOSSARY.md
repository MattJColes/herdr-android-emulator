# herdr-android-emulator

An Android emulator that a user and their coding agents share inside herdr: the user sees
and touches its screen, agents drive it and read what happened on it. Terms match
herdr-ios-simulator's wherever the platforms allow.

## Devices

**AVD**:
A saved Android Virtual Device definition: hardware profile, system image and disk. It can
be booted into a Device; on its own it does nothing.
_Avoid_: emulator, image, profile

**Default AVD**:
The AVD that `install-sdk` creates, `herdr-default`, booted when a Device is needed and
none is running.
_Avoid_: stock emulator, base image

**Device**:
A running Android instance that adb can reach, named by its serial (`emulator-5554`).
Booting an AVD gives a Device. Not a physical phone (yet).
_Avoid_: emulator, phone, instance

**Current Device**:
The one Device every agent and the View act on unless a command passes `--device`. Shared,
not per agent. `boot` sets it.
_Avoid_: default device, active device

**Supervisor**:
The background process that owns one Device for its whole life, keeping its screen stream
and its log entries while no View or command is using it.
_Avoid_: daemon, server, collector

## Screen

**View**:
The herdr pane showing the Current Device's live screen and taking the user's taps and
keys. Closing it stops the video, not the Device.
_Avoid_: pane, window, mirror

**Element**:
One labelled or identified item in the Device's UI hierarchy, numbered by the last `ui`.
_Avoid_: node, widget

## What happened

**Current app**:
The app the plugin last launched, by package name. `logs` and `terminate` default to it.
An app launched from Android Studio never becomes the Current app.
_Avoid_: target app, foreground app

**Event**:
One entry on a Device's timeline, written by the plugin: boot, shutdown, install, launch,
terminate, deep link, build, tap, mark, or a user tap or swipe in the View. Only actions
taken through this plugin become Events.
_Avoid_: lifecycle event, activity

**Log entry**:
One line from the Device's logcat. The app writes Log entries; the plugin writes Events.
_Avoid_: log line, logcat message

**Mark**:
A named point in time on a Device's timeline. `--since <mark>` reads from there.
_Avoid_: marker, checkpoint, bookmark

**Crash**:
A process on the Device that died: a Java or Kotlin exception, or a native signal with a
tombstone.
_Avoid_: error, failure, exception

**ANR**:
An app that stopped responding and that Android offered to kill. Not a Crash: the process
may still be alive and often recovers.
_Avoid_: freeze, hang, crash
