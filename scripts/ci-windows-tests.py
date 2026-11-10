#!/usr/bin/env python3
"""Run the existing Windows test selection from one workspace feature graph."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys

spec = importlib.util.spec_from_file_location('ci_tests', Path(__file__).with_name('ci-tests.py'))
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)
binaries, _ = ci.discover()
selected = ci.windows_jobs(binaries)
failed = False
for label, executable, cwd in selected:
    # Default listing includes ignored tests; preserve both runs for Slang.
    names = set(ci.listed_tests(executable, cwd))
    ignored = set(ci.ignored_tests(executable, cwd))
    print('COVERAGE ' + json.dumps({'target': label, 'normal': sorted(names - ignored),
                                    'ignored': sorted(ignored) if label.startswith('vork-slang/') else []}), flush=True)
    for flags in ([[], ['--ignored']] if label.startswith('vork-slang/') else [[]]):
        result = subprocess.run([executable, *flags, '--show-output'], cwd=cwd)
        failed |= bool(result.returncode)
# Cargo's no-run JSON has no doctest binaries; the original Slang command ran these too.
result = subprocess.run(['cargo', 'test', '--locked', '--release', '--workspace',
                         '--features', 'vork-render/test-support', '--doc'], cwd=ci.ROOT)
failed |= bool(result.returncode)
sys.exit(int(failed))
