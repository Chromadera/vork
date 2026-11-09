#!/bin/sh
set -eu
# SSH sessions do not necessarily inherit Dockerfile ENV values.
export PATH="/opt/vork:/opt/ffmpeg/bin:$PATH"
export LD_LIBRARY_PATH="/opt/ffmpeg/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
# The packaged engine discovers its companions and mounted GPU driver itself.
# Preserve explicit operator selections instead of replacing host ICD manifests.
exec "$@"
