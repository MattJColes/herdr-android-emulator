# gRPC for screen and input, with a token

Views read frames from the emulator's gRPC `streamScreenshot` and send taps and keys with
`sendTouch`/`sendKey`, about 30fps against about 2fps for `adb exec-out screencap`. This
brings tokio, tonic and prost into a project with no async runtime (about 80 crates, about
40s of cold build); the emulator's `.proto` files are vendored and `protoc-bin-vendored`
means no one installs protoc. The emulator always starts with `-grpc-use-token`, because
an unauthenticated port lets any local process drive a phone that may be signed in to
real accounts.

## Considered Options

- **`adb screencap` and `adb shell input`.** No new dependencies; too slow to use by hand.
- **scrcpy.** Faster still, but another tool for users to install and keep in step.

## Consequences

Physical devices have no emulator gRPC, so they fall back to screencap when they're added.
