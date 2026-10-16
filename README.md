# PocketPlay

Mirror and control an Android phone from Windows over USB or paired Wi-Fi. Create your own keyboard and mouse controls while apps and games continue running on the phone.

PocketPlay uses a Rust desktop application and a Rust Android capture service. A small Java launcher starts the device service, and JNI provides access to Android framework APIs. FFmpeg and SDL3 handle video decoding and presentation on Windows. The application does not use the scrcpy client/server, Qt, or a Python runtime.

## Features

- USB connections and Android Wireless debugging pairing.
- Custom keymap profiles with tap/hold buttons, joysticks, mouse look, and swipe controls. New profiles start empty.
- Adjustable capture resolution, bitrate, FPS target, codec, and hardware decoder.
- Fullscreen playback, aspect-ratio-aware resizing, rotation handling, and optional VSync.
- Phone-only audio, computer-only audio, or playback on both devices.
- Configurable audio prebuffer and phone display controls.
- English interface with Midnight, Light, and Ocean themes.

## Requirements

- Windows 10/11, x64. A Direct3D 11-capable GPU is recommended for hardware decoding.
- An ARM64 Android phone with USB debugging or paired Wireless debugging enabled.
- A data-capable USB cable, or a suitable local Wi-Fi connection.

The main validation device is a Xiaomi 12S Pro running Android 15 / HyperOS. The service uses Android framework APIs whose availability can differ across Android versions and manufacturers. Compatibility with every phone is not guaranteed.

## Getting started

1. Download a packaged Windows build from this repository's Releases page, when available, and extract the entire archive.
2. Enable USB debugging on the phone, connect it, unlock it, and accept the computer's authorization prompt. Xiaomi devices may also require **USB debugging (Security settings)** for touch injection.
3. Open `PocketPlay.exe`. In **Connect**, choose USB, refresh the device list, select the phone, and start a session.
4. Open **Keymap Studio** to create and position your controls, then save the profile.
5. In the viewer, press the backtick key (`` ` ``), beside `1`, to switch between keymap mode and normal mouse touch mode.

For Wi-Fi, enable **Wireless debugging** in Android Developer options. Pair using the pairing address and code, then connect using the main Wireless debugging address. USB generally offers more consistent delivery.

Keep the `tools/native` directory beside the executable. The source archive in this repository is not a ready-to-run Windows build; see [BUILDING.md](BUILDING.md).

## Stream settings

New settings use **H.264, 1920 px maximum edge, 8 Mbps, and a 60 FPS capture target**. FPS remains adjustable. The FPS target controls capture, not the game's graphics or rendering rate.

| Preset | Maximum edge | Video bitrate | FPS target |
| --- | --- | --- | --- |
| Low latency | 1920 px | 8 Mbps | 60 |
| Smooth | 1280 px | 5 Mbps | 60 |
| Quality | Native resolution | 16 Mbps | 60 |

Low latency also enables fullscreen, disables VSync, and keeps the phone display on. Smooth and Quality preserve those display choices. Video and audio changes apply on the next session.

VSync can improve presentation consistency at the cost of additional delay. Hardware decoding, USB, and fullscreen are useful starting points when prioritizing responsiveness. Actual latency and FPS depend on the source app, encoder, connection, GPU, and monitor. The FPS counters do not measure physical monitor scanout or the game's FPS, and zero latency is not possible.

## Audio and phone display

Audio uses uncompressed 48 kHz stereo PCM. The added audio prebuffer defaults to 0 ms; capture, transport, Windows, and the audio device can still introduce delay. Some apps restrict audio capture.

On supported Xiaomi/Redmi/Poco firmware, the screen-dark option disables the screen light while retaining physical refresh timing. This avoids the observed 60 Hz capture fallback caused by fully powering off the panel, but consumes more power than full panel power-off. Other devices use the available panel-power path.

The service keeps an already-awake session awake while streaming and restores the display on normal teardown. Manually locking the phone remains controlled by Android; PocketPlay does not bypass the lock screen.

## Project layout

```text
src/                  Windows UI, input, ADB, protocol, and viewer
android/native/       Rust Android capture and control service
android/src/          Minimal Java launcher
media/                C ABI glue and required public library headers
assets/               Application icon, manifest, and Windows resources
tools/native/         Runtime dependency staging directory
```

Build outputs, runtime binaries, device logs, personal keymaps, SDKs, and development recordings are excluded from this source repository. Keep downloadable application packages in GitHub Releases.

## Settings and troubleshooting

Settings and keymaps are stored in `%LOCALAPPDATA%\PocketPlayNative`, outside the application directory. Device and viewer diagnostics are saved as `native-device.log` and `native-viewer.log` in that directory. Review logs before sharing them; they can contain device details.

- **Device missing:** unlock the phone, check the data cable/driver, accept USB debugging, then refresh.
- **Video works but touch does not:** check touch-injection permissions and Xiaomi's debugging security option.
- **ADB connection issues:** the application's **Kill ADB server** button stops the shared host server; refreshing starts it again. Other ADB-based apps can be affected.
- **Unexpected FPS:** distinguish the capture target, incoming capture FPS, and viewer submissions. A static source can emit fewer frames.

## Development and licensing

See [BUILDING.md](BUILDING.md) for dependencies and build instructions, [ARCHITECTURE.md](ARCHITECTURE.md) for the design, and [SECURITY.md](SECURITY.md) for security considerations.

PocketPlay's application code is licensed under the [MIT License](LICENSE). Third-party headers and runtime components retain their own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
