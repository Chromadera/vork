#!/usr/bin/env python3
"""Linux VORK_SLANGC wrapper recording per-invocation compiler peak RSS.

Set VORK_SLANG_MEMORY_COMPILER to the actual pinned compiler and
VORK_SLANG_MEMORY_REPORT to an external JSONL file. Run a fresh workspace build
with this script as VORK_SLANGC. Disable any compiler artifact cache; a cache hit
does not measure compilation. Admission-wrapper waits count toward wall time.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import time


def main():
    if sys.platform != "linux":
        raise SystemExit("compiler RSS measurement requires Linux wait4 (KiB)")
    import fcntl

    compiler = Path(os.environ["VORK_SLANG_MEMORY_COMPILER"]).resolve()
    report = Path(os.environ["VORK_SLANG_MEMORY_REPORT"])
    if compiler == Path(__file__).resolve():
        raise SystemExit("measurement compiler must not be this wrapper")
    args = sys.argv[1:]
    if args in (["-version"], ["--version"], ["-help"], ["--help"]):
        os.execv(str(compiler), [str(compiler), *args])
    started = time.monotonic()
    process = subprocess.Popen([str(compiler), *args])
    _, status, usage = os.wait4(process.pid, 0)
    process.returncode = os.waitstatus_to_exitcode(status)

    def option(name):
        if name not in args:
            return None
        index = args.index(name) + 1
        return args[index] if index < len(args) else None

    row = dict(compiler=str(compiler), cwd=os.getcwd(), args=args,
               entry=option("-entry"), target=option("-target"), output=option("-o"),
               returncode=process.returncode, peak_rss_kib=usage.ru_maxrss,
               elapsed_seconds=time.monotonic() - started,
               user_seconds=usage.ru_utime, system_seconds=usage.ru_stime)
    report.parent.mkdir(parents=True, exist_ok=True)
    with report.open("a") as stream:
        fcntl.flock(stream, fcntl.LOCK_EX)
        stream.write(json.dumps(row) + "\n")
    return process.returncode if process.returncode >= 0 else 128 - process.returncode


if __name__ == "__main__":
    sys.exit(main())
