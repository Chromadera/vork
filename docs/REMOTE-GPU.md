# Run on your own NVIDIA GPU

Rendering requires Apple Silicon or an NVIDIA GPU with Vulkan ray queries.
Authoring scenes and `vork help` do not need a GPU. Docker requires a suitable
GPU on its host; it does not provide one. Hosted rendering: details will be
added here when available.

The Docker runtime below targets your own Linux x86-64 NVIDIA machine. Use a
matching Linux release archive and SHA256 from its `SHA256SUMS`. There is no
official registry image to pull yet. See [BUILD.md](BUILD.md) for source builds.
Put scene.json and all its assets in the job directory, keeping relative paths.

## Docker on an NVIDIA host

Use Linux x86-64 with a supported NVIDIA driver and the
[NVIDIA Container Toolkit](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/latest/install-guide.html).
The host supplies its NVIDIA libraries; do not install a mismatched driver in
the image. Docker Desktop on a Mac does not expose Metal to a Linux container.
WSL2 GPU support is a separate host configuration and has not been validated here.

```sh
python3 tools/gpu/prepare.py --bundle /path/to/vork-0.2.0-linux-x64.tar.gz \
  --sha256 RELEASE_SHA256 --context /tmp/vork-image-context
docker build --platform linux/amd64 -t vork-gpu:local /tmp/vork-image-context
docker run --rm --gpus all vork-gpu:local vork setup
docker run --rm --gpus all \
  -v /absolute/job:/job:ro -v /absolute/results:/results \
  vork-gpu:local vork render scene.json --out /results/video.mp4 --profile std/draft
```

Create the results directory first. The image runs as root by default; use
`--user "$(id -u):$(id -g)" -e VORK_CACHE_DIR=/tmp/vork-cache` on Linux
for your own output ownership. It needs a
writable `/tmp` and cache directory. Mount a writable directory and set
`VORK_CACHE_DIR` to reuse caches across containers; never share a writable cache
between mutually untrusted users.

The entrypoint creates a private temporary manifest pointing at
`libEGL_nvidia.so.0` and sets both `VK_DRIVER_FILES` and `VK_ICD_FILENAMES`.
A read-only NVIDIA manifest can point at `libGLX_nvidia.so.0`, which fails
in its headless environment. Rewriting `/etc/vulkan/icd.d` can also fail because
it is mounted read-only. This image leaves it alone. Do not bypass the entrypoint
when rendering. NVIDIA graphics, compute, utility and video capabilities are
requested; native NVENC handles MP4 encoding. FFmpeg is built with GPL/nonfree
features disabled and shared LGPL libraries, with matching source and notices
retained in `/opt/ffmpeg/notices`. There is no libx264 fallback in this image;
a working NVIDIA video encoder is required for H.264 video. This version's
`vork setup` can warn that libx264 is missing alongside its muxing message;
libx264 is not needed for native NVENC muxing. Setup exits successfully without libx264. Slang, OIDN, portable
shaders, the Preview License and all release notices accompany the executable.
The GPL Blender bridge is excluded. Ubuntu OS utilities retain their own
licenses, including GPL; the FFmpeg license check applies to its configured
binaries.
