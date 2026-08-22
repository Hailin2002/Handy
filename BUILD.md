# Build Instructions

This guide covers how to set up the development environment and build Handy from source across different platforms.

## Prerequisites

### All Platforms

- [Rust](https://rustup.rs/) (latest stable)
- [Bun](https://bun.sh/) package manager
- [Tauri Prerequisites](https://tauri.app/start/prerequisites/)

### Platform-Specific Requirements

#### macOS

- Xcode Command Line Tools
- Install with: `xcode-select --install`

##### Intel Mac (x86_64)

Prebuilt ONNX Runtime binaries are not available for Intel Macs. Install ONNX Runtime via Homebrew and link dynamically:

```bash
brew install onnxruntime
ORT_LIB_LOCATION=$(brew --prefix onnxruntime)/lib ORT_PREFER_DYNAMIC_LINK=1 bun run tauri dev
```

The same environment variables apply for production builds:

```bash
ORT_LIB_LOCATION=$(brew --prefix onnxruntime)/lib ORT_PREFER_DYNAMIC_LINK=1 bun run tauri build
```

#### Windows

- Microsoft C++ Build Tools: Visual Studio 2019/2022 with C++ development
  tools, or Visual Studio Build Tools 2019/2022
- [CMake](https://cmake.org/download/) (must be on `PATH`):

  ```powershell
  winget install Kitware.CMake
  ```

- [Vulkan SDK](https://vulkan.lunarg.com/sdk/home) from LunarG — required to
  build the Vulkan GPU backend (`vulkan-shaders-gen` needs the SDK's headers
  and `glslc`):

  ```powershell
  winget install KhronosGroup.VulkanSDK
  ```

  Open a new terminal afterward so `VULKAN_SDK` is set.

> [!NOTE]
> Windows' 260-character path limit used to break the native Vulkan build in
> most checkouts. Since `transcribe-cpp` 0.1.3 the build works around it
> automatically (it compiles through a short NTFS junction — no admin rights
> or setup needed), so a normal checkout just builds. If you still hit
> path-limit errors, see
> [Windows build fails with path-limit errors](#windows-build-fails-with-path-limit-errors-msb3491--ftk1011--msb6003)
> in Troubleshooting.

#### Linux

- Build essentials
- ALSA development libraries
- Install with:

  ```bash
  # Ubuntu/Debian
  sudo apt update
  sudo apt install build-essential clang libclang-dev libevdev-dev libasound2-dev pkg-config libssl-dev libvulkan-dev vulkan-tools glslc spirv-headers glslang-tools libgtk-3-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libgtk-layer-shell0 libgtk-layer-shell-dev patchelf cmake

  # Fedora/RHEL
  sudo dnf groupinstall "Development Tools"
  sudo dnf install alsa-lib-devel pkgconf openssl-devel vulkan-devel glslc \
    clang clang-devel libevdev-devel \
    spirv-headers-devel spirv-tools-devel glslang \
    gtk3-devel webkit2gtk4.1-devel libappindicator-gtk3-devel librsvg2-devel \
    gtk-layer-shell gtk-layer-shell-devel \
    cmake

  # Arch Linux
  sudo pacman -S base-devel clang libevdev shaderc spirv-headers glslang alsa-lib pkgconf openssl vulkan-devel \
    gtk3 webkit2gtk-4.1 libappindicator-gtk3 librsvg gtk-layer-shell \
    cmake
  ```

## Setup Instructions

### 1. Clone the Repository

```bash
git clone git@github.com:cjpais/Handy.git
cd Handy
```

### 2. Install Dependencies

```bash
bun install
```

### 3. Start Dev Server

```bash
bun tauri dev
```

### 4. Build for Production

```bash
bun run tauri build
```

This compiles a release binary and generates platform-specific bundles (deb, rpm, AppImage on Linux; dmg on macOS; msi on Windows).

## Linux Install (from source)

The raw binary (`src-tauri/target/release/handy`) cannot run standalone — it needs Tauri resource files (tray icons, sounds, VAD model) to be co-located at the expected path.

**Install from the deb bundle** (works on any Linux distro):

```bash
cd /tmp
ar x /path/to/Handy/src-tauri/target/release/bundle/deb/Handy_*_amd64.deb data.tar.gz
tar xzf data.tar.gz
sudo cp usr/bin/handy /usr/bin/
sudo cp -a usr/lib/. /usr/lib/
sudo cp -r usr/share/icons/hicolor/* /usr/share/icons/hicolor/
sudo cp usr/share/applications/Handy.desktop /usr/share/applications/
```

The runtime libraries live in the app-private `/usr/lib/Handy/` (on the binary's rpath), so no `ldconfig` step is needed.

After subsequent rebuilds, copy the binary and any refreshed runtime libraries:

```bash
sudo cp src-tauri/target/release/handy /usr/bin/
sudo mkdir -p /usr/lib/Handy
sudo cp -a src-tauri/transcribe-libs/. /usr/lib/Handy/
```

Resources only need re-copying if they change upstream (new icons, sounds, models, etc.).

## Troubleshooting

### macOS Accessibility remains enabled after a local rebuild

Local builds use the ad-hoc `signingIdentity: "-"`. A rebuild can have a new macOS code
identity while the old **System Settings > Privacy & Security > Accessibility** entry
remains visibly enabled, leaving Handy on `Waiting...`.

After installing the final bundle at `/Applications/Handy.app`, quit Handy, clear only its
stale Accessibility record, then reopen it:

```bash
osascript -e 'tell application id "com.pais.handy" to quit' || true
tccutil reset Accessibility com.pais.handy
open /Applications/Handy.app
```

Grant Accessibility again when prompted. This does not reset Microphone or other TCC
services, and official releases normally do not need it.

For optional diagnosis, compare the designated requirements of the previous and rebuilt
bundles:

```bash
codesign -dr - /path/to/previous/Handy.app 2>&1
codesign -dr - /Applications/Handy.app 2>&1
```

An ad-hoc requirement contains a `cdhash`; a changed requirement confirms the rebuild is
not covered by the old grant. The reset procedure does not require this check.

See [issue #1618](https://github.com/cjpais/Handy/issues/1618) for the related onboarding
and stale-permission report.

### AppImage build fails on Arch / rolling-release distros

`linuxdeploy` bundles its own `strip` binary which is too old to process system libraries built with newer toolchains on rolling-release distros (Arch, CachyOS, Manjaro, EndeavourOS).

The error from Tauri:

```
Bundling Handy_*_amd64.AppImage
failed to bundle project `failed to run linuxdeploy`
```

Tauri swallows the real linuxdeploy error. To see it, run linuxdeploy manually:

```bash
cd src-tauri/target/release/bundle/appimage
~/.cache/tauri/linuxdeploy-x86_64.AppImage --appimage-extract-and-run \
  --appdir Handy.AppDir --plugin gtk --output appimage
```

**Workaround:** The binary, deb, and rpm bundles all build fine — only the AppImage step fails. To skip it:

```bash
bun run tauri build -- --bundles deb
```

Then install using the deb extraction method above.

### Windows build fails with path-limit errors (`MSB3491` / `FTK1011` / `MSB6003`)

On Windows the native build can fail partway through `transcribe-cpp-sys` with
any of these (all the same root cause):

```
error MSB3491: Could not write lines to file "...VCTargetsPath.tlog\VCTargetsPath.lastbuildstate".
Path: ... exceeds the OS max path limit. The fully qualified file name must be less than 260 characters.
```

```
FileTracker : error FTK1011: could not create the new file tracking log file:
...\vulkan-shaders-gen-build\...\cmTC_xxxxx.tlog\link.write.1.tlog.
The system cannot find the path specified.
```

```
error MSB6003: The specified task executable "CL.exe" could not be run.
System.IO.DirectoryNotFoundException: Could not find a part of the path ...
```

This is **not** a code or toolchain problem — it's Windows' legacy 260-character
path limit (`MAX_PATH`), overflowed by the Vulkan shader generator's nested
CMake build tree on top of Cargo's already-deep
`target\release\build\<crate>-<hash>\out\build\...` directory.

Since `transcribe-cpp` 0.1.3 this is mitigated automatically: the native build
compiles through a short NTFS junction under `%LOCALAPPDATA%\tcs` (created
without admin rights), so a normal checkout builds with no setup. Enabling
Windows long paths does **not** reliably help here — MSBuild's native
`FileTracker` (`tracker.exe`) ignores the long-paths flag — which is why the
junction, not the registry flag, is the fix.

If you still see the errors above, junction creation was likely blocked
(filesystem or corporate policy) — the failing build's log then contains a
`transcribe-cpp-sys: could not create short build junction ...` warning — or
your checkout is deep enough to overflow even the shortened layout. Work
around either case with a short Cargo target directory:

```powershell
# Per-shell:
$env:CARGO_TARGET_DIR = "C:\h"

# Or persist it for all future terminals (note: redirects ALL your
# Rust projects' build output, not just Handy):
[Environment]::SetEnvironmentVariable('CARGO_TARGET_DIR', 'C:\h', 'User')
```

Artifacts then land in `C:\h\release\...` instead of the repo's
`src-tauri\target\`. Open a **new terminal** if you persisted the variable —
it is only picked up by freshly started processes. Then `bun run tauri dev`
and `bun run tauri build` work normally.

On top of that workaround, this repository additionally enforces a
**serial** native build — see
[Windows native build fails silently inside MSBuild](#windows-native-build-fails-silently-inside-msbuild-transcribe-cpp-sys-serial-build-enforced)
below, which also covers the `VULKAN_SDK` requirement for non-inherited
shells.

### Windows native build fails silently inside MSBuild (`transcribe-cpp-sys`, serial build enforced)

Independently of the path-limit errors above, on some Windows machines the
cmake-driven native build of `transcribe-cpp-sys` (the Whisper/ggml tree,
including its nested `ggml`/Vulkan sub-builds) aborts with exit code 1 and
**no visible compiler error** as soon as MSBuild worker parallelism is
involved. A differential measurement against the same CMake build directory
showed:

| Invocation | Result |
| --- | --- |
| `cmake --build <dir> --target install --config Release` (serial) | succeeds, exit 0 |
| same command plus `--parallel 20` (what the build script issues by default) | fails, exit 1, right after `Checking File Globs`, zero diagnostics |

Two independent stressors were confirmed on this machine via an MSBuild
diagnostic log (`/v:diag /flp /m:2`):

1. **MSBuild worker nodes (`/m`) fail silently** — even `/m:1`. The nested
   `GetNativeManifest` sub-builds of the ggml tree end with exit 1 and no
   diagnostic output whenever worker parallelism is enabled; the identical
   tree builds serially.
2. **FileTracker `MAX_PATH` overflow** — Windows' `LongPathsEnabled`
   registry flag is ignored by MSBuild's FileTracker for `.tlog`
   full-path computation (`FTK1011`), and the Vulkan `ExternalProject`
   try-compile scratch dirs under Cargo's deep `OUT_DIR` exceed the classic
   260-character limit.

#### What this repository does about it

Because Cargo always overwrites the build script's `NUM_JOBS` with its own
job count ([rust-lang/cargo#9972](https://github.com/rust-lang/cargo/issues/9972))
and upstream `cmake-rs` 0.1.x has no knob for it, the enforced serial build
is implemented with two committed config changes:

1. **Vendored `cmake-rs` 0.1.57** lives at [`vendor/cmake`](vendor/cmake)
   (with `src/lib.rs` marked `LOCAL PATCH (Speak)`). It honors
   `CMAKE_BUILD_PARALLEL_LEVEL` ahead of `NUM_JOBS` when passing
   `--parallel <N>` to `cmake --build`, and additionally relocates long
   Windows build directories (>120 chars) behind a short NTFS junction in
   `%TEMP%\cmake-rs-short-build\<hash>` to defuse the FileTracker
   `MAX_PATH` issue. It is wired up via the `[patch.crates-io]` table in
   [`src-tauri/Cargo.toml`](src-tauri/Cargo.toml) (~line 194):
   `cmake = { path = "../vendor/cmake" }`.
2. **Pinned parallelism** via the `[env]` table in
   [`.cargo/config.toml`](.cargo/config.toml) (line 11):
   `CMAKE_BUILD_PARALLEL_LEVEL = "1"`, so every `cargo` invocation from the
   repository root builds the native tree deterministically serially.

Both files carry comments explaining the rationale; raise the value only
after re-verifying MSBuild parallelism works on your machine.

**Tradeoff:** the native `transcribe-cpp-sys` compilation is single-threaded,
which makes it noticeably slower than an out-of-the-box parallel build. This
is deliberate: a slow-but-reliable build was preferred over a fast one that
fails without diagnostics on affected machines. Machines where MSBuild
parallelism works can set `CMAKE_BUILD_PARALLEL_LEVEL` to their core count
in `.cargo/config.toml`.

#### Requirements for the native build

- **CMake** must be on `PATH` (see Prerequisites above).
- **Vulkan SDK**: a fresh CMake configure pass needs `VULKAN_SDK` (or
  `glslc`) in the environment or `FindVulkan` fails with
  `Could NOT find Vulkan`. Normally installed user shells inherit the
  machine-wide `VULKAN_SDK` variable set by the SDK installer. Restricted
  shells that do **not** inherit machine environment variables (CI steps,
  sandboxed harnesses, custom service accounts) must set it explicitly,
  e.g.:
  ```powershell
  $env:VULKAN_SDK = 'C:\VulkanSDK\1.4.357.0'   # adjust to your SDK version
  $env:PATH    = "$env:VULKAN_SDK\Bin;$env:PATH"
  ```
- **Junction write access**: `transcribe-cpp-sys` creates its own short-path
  junction under `%LOCALAPPDATA%\tcs\<hash>`, and the vendored cmake-rs
  creates `%TEMP%\cmake-rs-short-build\<hash>` junctions. If either is
  blocked by permissions/policy you'll see a corresponding
  `cargo:warning=... could not create short build junction ...`; the build
  then continues through the deep `OUT_DIR` and can hit the `MAX_PATH`
  errors described above.

#### Verifying your setup

From the repository root, a plain:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml
```

picks up `.cargo/config.toml` automatically (Cargo discovers `.cargo/`
config files by walking up from the **current working directory**, so run
this from the repository root). On the reference machine this
was verified green after the fix: **208 tests passed** in `speak_app_lib`
plus **12 tests** in `meeting_loopback_spike`, 0 failures;
`cargo clippy --all-targets` passed with 0 warnings; `cargo fmt --check`
passed. The raw verification logs are recorded in
[`docs/meeting-mode-phase0-baseline.md` section 2.3](docs/meeting-mode-phase0-baseline.md)
(log paths `src-tauri/target/baseline-tools/logs/11-cargo-test-serial-fix.log`
and `13-`/`14-cargo-clippy-final.log`; the `target/` directory itself is not
committed).

If `cargo test` still fails with the silent-exit pattern, first confirm no
environment variable overrides `CMAKE_BUILD_PARALLEL_LEVEL`
(`Get-ChildItem Env:CMAKE_BUILD_PARALLEL_LEVEL` should print the value from
`.cargo/config.toml` unless you deliberately changed it), then re-run with
`cargo test -vv` to capture the cmake output for a bug report.

### Windows `tauri build` fails at bundling with `program not found`

If the build compiles all the way to `Built application at: ...\handy.exe` and
then fails with:

```
Signing C:\...\handy.exe with a custom signing command
failed to bundle project `program not found`
```

that's the code-signing step: `tauri.conf.json` configures a custom
`signCommand` (`trusted-signing-cli`, Azure Trusted Signing) that only exists
in the release CI environment. Local development doesn't need it:

```powershell
# Development (no bundling/signing at all):
bun run tauri dev

# Or compile a release binary without the installer/signing step:
bun run tauri build --no-bundle
```
