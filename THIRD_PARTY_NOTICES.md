# Third-party notices

PocketPlay's own Rust, Java, and C glue code is MIT licensed; see LICENSE.

## Headers included in this repository

- SDL3 public headers: zlib license. See `licenses/SDL-LICENSE.txt` and the notices in the headers. Upstream: https://github.com/libsdl-org/SDL
- FFmpeg public headers: retain their upstream license notices. License texts are in `licenses/FFmpeg/`. Upstream: https://ffmpeg.org/

Keep the upstream notices when redistributing these headers. Rust dependencies are identified and pinned by the two Cargo.lock files; their licenses are declared in the upstream crates.

## Runtime dependencies not included in the source tree

- Android Debug Bridge: Android Open Source Project, Apache-2.0. https://android.googlesource.com/platform/packages/modules/adb/
- SDL3: zlib license. https://github.com/libsdl-org/SDL
- FFmpeg: license terms depend on the actual build configuration. The existing development bundle used a build reporting FFmpeg 9.0.2 without GPL enabled. https://ffmpeg.org/legal.html
- Any additional libraries linked into your chosen runtime builds retain their own licenses.

The original development media binaries came from the official scrcpy 5.0 Windows bundle. PocketPlay does not use its client or server. Provenance: https://github.com/Genymobile/scrcpy/releases/tag/v5.0

Binary release maintainers must include the applicable notices and source information for the binaries they actually distribute. This source export does not include third-party DLLs, ADB executables, or a corresponding-source archive for a binary release.
