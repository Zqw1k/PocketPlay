# Architecture

## Windows

The Rust application uses egui/eframe for configuration and keymap editing. The ADB layer discovers devices, pairs Wi-Fi connections, deploys a session service, and owns the forwarding rules it creates. Keyboard and mouse mappings produce touch events for Android.

The viewer decodes compressed reference frames in order and retains only the latest decoded frame awaiting presentation. FFmpeg decoding and SDL3/D3D11 rendering are exposed through a small C ABI shim in `media/engine.c`. Hardware decoding shares the renderer's D3D11 device. Aspect fitting and touch-coordinate mapping use the same logical content rectangle.

## Android

A minimal Java launcher starts a Rust shared library through app_process under the authorized ADB identity. JNI is used for Android framework objects, display geometry, input injection, and display controls. Hardware capture/encoding uses an input surface and NDK MediaCodec. Audio capture uses Android's available playback-capture paths.

Video, audio, and control use separate authenticated connections over ADB forwarding. A random session token and peer identity checks restrict the device service. Geometry changes reconfigure the video path; stable physical refresh-mode changes can rebind the mirror without resetting input state.

## Session lifecycle

The desktop prepares its decoder and renderer before starting video capture. Startup waits for the first presented frame before reporting a connection and applying screen-dark mode. Owned resources, display restoration, and a session wake lock are released during normal teardown and handled disconnects.

Xiaomi screen-dark mode retains physical timing while disabling display light. This differs from full panel power-off and has a power cost. Abrupt service termination can prevent brightness restoration. Manual phone locking remains controlled by Android.
