# Release packaging

No script here uploads or publishes. Archives include the root `LICENSE`, which states the terms.
Third-party licences apply independently.

## Notices

Run `cargo fetch --locked`, then `scripts/generate-notice` after Cargo.lock changes.
`--check` fails on drift or missing licence text. The generator includes every locked
external package (all targets, including build dependencies), native vendored header
notices, LTC/three.js, NVENC, Blender OCIO/AgX attribution and asset licences.
`cargo deny check licenses` checks the SPDX allowlist.

Some published crates omit their licence files. `licenses/crates/` preserves the upstream
texts for these crates; sources.json pins their locked versions and regeneration fails until those supplements are reviewed for an update. The objc2 family
uses the repository-root LICENSE.md; gpu-descriptor uses license/{MIT,APACHE}; r-efi
uses the exact published AUTHORS file. Basis Universal and Intel ISPC have explicit
upstream texts alongside the generated header notices. AgX's OCIO text is from Blender
commit 3237ae2855b6dbfad8ba9e86e6e7add0942403d7 (`release/text/ocio-license.txt`).
These notices do not settle the pending owner review of AgX attribution.

## Archive platforms

Build `packages/ts` with `npm ci && npm run build`, then:

Prepare the runtime shader bundle with the exact binary on a supported GPU of
the target backend (Vulkan for Linux/Windows, Metal for macOS). This runs setup
with `--all` on the packaging machine and exports only portable compiler artifacts;
it never distributes that machine's driver cache or warmup ledger. Preparation
verifies each entry's BLAKE3 cache envelope before creating the manifest and
requires the pinned Python `blake3` package below. A valid duplicate replaces a
corrupt entry from another store; a corrupt entry with no valid duplicate fails
preparation.

```
uv run --with blake3==1.0.8 python scripts/prepare-shaders --binary PATH_TO_BINARY --output SHADER_BUNDLE
```

Pass `--shader-cache SHADER_BUNDLE` to `package-release`. A `shaders` directory
beside the binary is also picked up automatically. Packaging checks the binary
and every asset against the bundle manifest. Builds without a prepared bundle
retain runtime compilation. Uncovered authored programs and resource layouts
always compile on demand. Slang still ships for that work. SPIR-V and MSL avoid
Slang compilation on the user's machine; the Vulkan/Metal driver still compiles
them for its GPU. The hosted artifact workflows do not prepare GPU bundles;
prepare them on the target GPU before producing a release archive.

```
scripts/package-release --binary PATH_TO_BINARY --platform linux-x64 \
  --output OUTPUT --shader-cache SHADER_BUNDLE --oidn-toolchain PATH_TO_OIDN --slang-toolchain PATH_TO_PINNED_SLANG
scripts/package-release --binary PATH_TO_BINARY --platform darwin-arm64 \
  --output OUTPUT --shader-cache SHADER_BUNDLE --oidn-toolchain PATH_TO_OIDN --slang-toolchain PATH_TO_PINNED_SLANG
python scripts/package-release --binary target\release\vork.exe --platform windows-x64 \
  --output OUTPUT --shader-cache SHADER_BUNDLE --oidn-toolchain PATH_TO_OIDN --slang-toolchain PATH_TO_PINNED_SLANG \
  --msvc-runtime PATH_TO_VC_REDIST_CRT
```

The Windows archive is a zip with `vork.exe` and the same contents; `.github/workflows/windows-dist.yml`
builds it and uploads it with `install.ps1`. `vork.exe` and Slang link the C runtime statically. OIDN's
DLLs import the Visual C++ runtime, so the packager reads their import tables and copies exactly the
imported runtime DLLs (currently `msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`) from
`--msvc-runtime` (Visual Studio's `VC\Redist\MSVC\<version>\x64\Microsoft.VC14x.CRT`) into `oidn/bin`,
so denoising needs no system-wide VC++ Redistributable. Terms: `licenses/msvc-runtime.txt`.
Building on Windows needs Visual Studio Build Tools (C++), Rust 1.96 and LLVM (libclang for
basisu bindgen).

Install the pinned toolchain with `scripts/install-slang DESTINATION`. Every platform
requires `--oidn-toolchain PATH_TO_OIDN --slang-toolchain DESTINATION`; packaging fails without it or on a version
mismatch. `slang/` ships slangc, its compiler library, standard modules (plus the SPIR-V optimizer
on Linux and Windows) and Slang's notices; `oidn/` ships OIDN's runtime libraries and notices.
Run `scripts/check-release ARCHIVE` on the target platform with a Vulkan/Metal device
(software Vulkan is sufficient). It runs the installer, setup, and separate program and OIDN scenes
from the installed tree with a fresh cache, `VORK_SLANGC` unset and an empty PATH. Linux and Windows
artifact workflows run this check before uploading.

Use the matching binary for each platform; the packager does not cross-compile. Both
archives include NOTICE, selected project licence texts, third-party licences, examples
and licensed assets, a local ESM SDK and the Python SDK (no npm publication required).

## Installer

`install.sh` uses curl and SHA256SUMS, with no gh or sudo. It supports VERSION,
PREFIX, VORK_REPO and VORK_DOWNLOAD_BASE (for local fixture testing).
It swaps the new tree in whole, so upgrades drop removed files, and at the default
prefix it adds `bin` to the shell startup file (`VORK_NO_MODIFY_PATH=1` opts out).
`--dry-run` prints exact release
URLs and destinations without downloading, authenticating or writing files.

`install.ps1` is the Windows equivalent (`-DryRun`, the same environment
variables, PREFIX defaulting to `%LOCALAPPDATA%\Programs\vork`).

`python3 scripts/test-installer.py` exercises checksum corruption, missing/duplicate
checksums, traversal rejection, installation and token-free dry runs for the installer of the host platform;
on Windows, `VORK_TEST_BINARY=path\to\vork.exe` adds a real install and setup.

OIDN is pinned in `dist/oidn.json`. Install with `scripts/install-oidn DEST`
(or `--platform linux-x64|darwin-arm64|windows-x64` for cross packaging), then
pass `--oidn-toolchain DEST`. Packages retain every OIDN runtime library (CPU/GPU
backends, built-in models) and upstream notices, not its sample tools, headers or manuals. Development runs may set `VORK_OIDN`
to the full native library path. The library is loaded for an enabled denoiser (on by default in std/final and std/path).
If initialization fails, rendering continues without OIDN and prints a warning.
