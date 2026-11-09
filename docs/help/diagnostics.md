Setup and render progress goes to stderr: a plain line every 5 seconds in terminals and redirected logs. Redirect both streams (`>run.log 2>&1`) for a complete log. Counts cover completed configurations/frames and compiler calls discovered so far; elapsed time is not an ETA.
DIAGNOSTICS: --diagnostics writes NAME.timings.json (adapter, cache, GPU, encoder timings) and
non-finite pixel counts; with it, --no-write (no files) and --gpu-only (no readback) benchmark.
Environment: VORK_CACHE_DIR, VORK_CACHE_BYTES (default 16 GiB; 0 = no disk cache), VORK_SLANGC,
VORK_OIDN, VORK_BIN (SDK executable).

Diagnostics JSON uses version 3 and numeric evaluation seconds for frame times;
output FPS remains number/rational string. See vork help outputs for completion
metadata and overwrite ownership.
