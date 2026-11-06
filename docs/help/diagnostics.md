DIAGNOSTICS: --diagnostics writes NAME.timings.json (adapter, cache, GPU, encoder timings) and
non-finite pixel counts; with it, --no-write (no files) and --gpu-only (no readback) benchmark.
Environment: VORK_CACHE_DIR, VORK_CACHE_BYTES (default 16 GiB; 0 = no disk cache), VORK_SLANGC,
VORK_OIDN, VORK_BIN (SDK executable).
