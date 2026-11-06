# Build from source

Run these commands from the checkout root. Source builds need Rust, a native
C/C++ toolchain, Python with safe tar extraction filters, and the pinned Slang
compiler. Cargo does not download Slang. The supported Unix toolchain archives
are Apple Silicon macOS and x86-64 Linux (see `dist/slang.json`); there is no
pinned Intel Mac or Linux ARM archive.

## Prerequisites

On macOS, install Apple's command line tools, then install Homebrew from
https://brew.sh if it is not already available:

```sh
xcode-select --install
```

Finish that installer before continuing. Use Homebrew Python explicitly:
stock macOS Python 3.9 may lack the extraction filters required by the Slang
installer. No pip packages are needed.

```sh
brew install python@3.12
PYTHON="$(brew --prefix python@3.12)/bin/python3.12"
```

On Ubuntu 24.04 x86-64 (the Linux baseline), install:

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libclang-dev curl git python3 ca-certificates
PYTHON=python3
```

Basis texture bindings require libclang (`libclang-dev` on Ubuntu; supplied
by Apple's command line tools on macOS). If bindgen cannot find it, set
`LIBCLANG_PATH` to the directory containing libclang.

Other Linux distributions need equivalent C/C++ build tools, libclang and Python 3.12+
(or a Python release with backported `tarfile.data_filter`). Rendering also
needs a working Vulkan driver; building and GPU-free commands do not need a GPU.

Install Rust 1.96, matching CI, through rustup (https://rustup.rs):

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/vork-rustup.sh
sh /tmp/vork-rustup.sh -y --default-toolchain 1.96
. "$HOME/.cargo/env"
```

## Install Slang and build

Clone the repository if needed, then enter it.

Choose a new, empty destination outside the checkout. The installer downloads
and verifies the archive against `dist/slang.json`, and refuses to overwrite
an existing directory. Keep the entire toolchain, including its libraries,
modules and notices, together.

```sh
"$PYTHON" scripts/install-slang "$HOME/.local/share/vork/slang-2026.20"
export VORK_SLANGC="$HOME/.local/share/vork/slang-2026.20/bin/slangc"
"$VORK_SLANGC" -version
cargo +1.96 build --release --workspace --locked
./target/release/vork --help
./target/release/vork help
```

The build compiles shaders for both Vulkan and Metal on either host. It checks
the Slang version against the pin. `VORK_SLANGC` must be an absolute executable
path; alternatively the build can find the pinned `slangc` on `PATH`. Export
`VORK_SLANGC` again in each new shell, or add the export to your shell startup
file. A `cannot start slangc` error means this setup is missing; a version
mismatch means a different compiler was selected.

The binary is `target/release/vork`, or `$CARGO_TARGET_DIR/release/vork` when
that environment variable is set. These commands do not install it globally.
Runtime program compilation also needs Slang; retain the export when running
the binary from the build directory.

## Windows x64

Install Rust 1.96 through rustup, Visual Studio Build Tools with Desktop development
with C++, LLVM (libclang for bindgen), and Python 3.12+. In a Developer PowerShell,
from the checkout root:

```powershell
python scripts/install-slang "$env:LOCALAPPDATA/vork/slang-2026.20"
$env:VORK_SLANGC = "$env:LOCALAPPDATA/vork/slang-2026.20/bin/slangc.exe"
cargo +1.96 build --release --workspace --locked
.\target\release\vork.exe help
```

Rendering requires a supported NVIDIA Vulkan driver on Windows and Linux, or
Apple Silicon Metal on macOS. For binary installation on Windows use install.ps1;
see README.md (Install). The compiled help and scene JSON are identical on every platform.

## Runtime dependencies

Video output needs FFmpeg and ffprobe on PATH.
Install the pinned denoiser with `python scripts/install-oidn DESTINATION`.
Set VORK_OIDN to its lib/libOpenImageDenoise.so.2 on Linux,
lib/libOpenImageDenoise.2.dylib on macOS, or bin/OpenImageDenoise.dll on Windows.

Node/npm and the SDKs are not prerequisites for the Rust workspace build.
For SDK setup use `vork help sdk`; for authored Slang use `vork help programs`.
