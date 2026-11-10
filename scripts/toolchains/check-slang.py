#!/usr/bin/env python3
"""Check upstream #13648 stays fixed in the installed official compiler."""
import argparse
from pathlib import Path
import re
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('compiler')
args = parser.parse_args()
source = Path(__file__).with_name('address-space-dedup.slang')
with tempfile.TemporaryDirectory() as directory:
    for target in ('spirv-asm', 'wgsl', 'metal'):
        output = Path(directory) / target
        subprocess.run([args.compiler, str(source), '-target', target,
                        '-entry', 'main', '-O2', '-o', str(output)], check=True)
        text = output.read_text()
        if target == 'spirv-asm':
            count = len(re.findall(r'OpName %\S+ "leaf"', text))
            assert count == 1, f'expected one leaf body, got {count}'
        assert text.strip(), f'empty {target} output'
        print(f'{target}: generated successfully')
