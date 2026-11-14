# SDL3 presentation queue patch

The Windows runtime includes SDL 3.4.18, built from upstream tag
`release-3.4.18` (commit `829a65d769d935c4852f8159e964312c0957260a`)
with `low-latency.patch` applied. SDL is distributed under its zlib license;
see `licenses/SDL-LICENSE.txt` in the runtime distribution.

The opt-in hint `SDL_RENDER_DIRECT3D11_LOW_LATENCY` enables a waitable
DXGI swap chain with a per-swap-chain maximum frame latency of one.
PocketPlay enables it when VSync is disabled. Unsupported swap-chain
creation retries with the standard flags. The patch does not add a
frame-rate limiter, change image quality, or request exclusive fullscreen.
No frame-latency wait handle is acquired by the patch.

## Rebuild

Use an x64 Windows compiler and SDK supporting DXGI 1.3, CMake and Ninja.
The measured build used LLVM-MinGW Clang 23.1.3, Release optimization.
From a clean checkout of the pinned SDL tag:

```powershell
git apply /path/to/low-latency.patch
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DSDL_SHARED=ON -DSDL_STATIC=OFF -DSDL_TESTS=OFF -DSDL_EXAMPLES=OFF
cmake --build build --parallel 4
```

Keep the resulting `SDL3.dll` in `tools/native/` beside the other runtime
libraries. Unmodified SDL 3.4.18 remains compatible and ignores the hint.
The measured gain concerns PC submission blocking at 120 FPS, not a
verified physical input-to-photon improvement or a guaranteed gain on all GPUs.

Upstream: https://github.com/libsdl-org/SDL/tree/release-3.4.18
DXGI reference: https://learn.microsoft.com/en-us/windows/uwp/gaming/reduce-latency-with-dxgi-1-3-swap-chains
