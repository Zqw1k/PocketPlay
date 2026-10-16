# Native engine safety notes

The device service runs with the authorized ADB shell identity. It does not root the phone or elevate an app UID. Android permissions remain enforced. Framework compatibility has been tested on Xiaomi 12S Pro / Android 15; hidden framework APIs can change on other versions.

Connections use loopback ADB forwarding to an abstract Unix socket. The service checks peer UID (adbd shell or root), a random 128-bit session token, channel type and bounded configuration. Ordinary phone app UIDs are rejected. Video packets are bounded at 16 MiB and decoded images at 8192 px per edge / 16 MP. Helper output, JSON imports and screenshot allocations retain their existing bounds.

Encoded video, input and audio use separate connections. The viewer retains only the newest decoded frame awaiting presentation. Compressed reference frames are decoded in order. The D3D11 device uses multithread protection, and native frame references remain owned until rendering completes.

Only owned helper processes and forwarding rules are stopped. A session-scoped Android wake lock retains an already-awake session without wake-up flags and is released on teardown. Production no longer changes the global charging stay-awake setting. Xiaomi screen-dark mode saves and restores display brightness while keeping physical timing active; other manufacturers retain panel-power restoration. Cleanup runs on normal shutdown and handled connection loss; abrupt process termination can prevent brightness restoration. No phone power-off command or lock-screen bypass is used.

Media DLLs are loaded by absolute path from the bundled native tools directory. Runtime code avoids unwrap/expect and propagates contextual errors. This is not a penetration-test report, and the Cargo audit does not scan native media DLLs or Windows/Android components.
