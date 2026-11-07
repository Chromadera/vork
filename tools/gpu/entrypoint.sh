#!/bin/sh
set -eu
# SSH sessions do not necessarily inherit Dockerfile ENV values.
export PATH="/opt/vork:/opt/ffmpeg/bin:$PATH"
export LD_LIBRARY_PATH="/opt/ffmpeg/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export VORK_SLANGC="${VORK_SLANGC:-/opt/vork/slang/bin/slangc}"
export VORK_OIDN="${VORK_OIDN:-/opt/vork/oidn/lib/libOpenImageDenoise.so.2}"
# Host/container runtimes can mount /etc/vulkan/icd.d read-only. Never edit it.
# The GLX ICD needs an X path unavailable in headless containers; EGL works there.
runtime=$(mktemp -d /tmp/vork-runtime.XXXXXXXX)
trap 'rm -rf "$runtime"' EXIT HUP INT TERM
chmod 700 "$runtime"
printf '%s\n' '{"file_format_version":"1.0.1","ICD":{"library_path":"libEGL_nvidia.so.0","api_version":"1.3.0"}}' > "$runtime/nvidia.json"
export VK_DRIVER_FILES="$runtime/nvidia.json" VK_ICD_FILENAMES="$runtime/nvidia.json" XDG_RUNTIME_DIR="$runtime"
"$@"
