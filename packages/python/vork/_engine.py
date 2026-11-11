"""Run the separately installed renderer through VORK_BIN or PATH."""
import os
import subprocess


def engine():
    return os.environ.get('VORK_BIN') or 'vork'


def run_engine(args, **kwargs):
    executable = engine()
    try:
        return subprocess.run([executable, *args], **kwargs)
    except FileNotFoundError:
        raise RuntimeError(
            f"vork executable not found: {executable!r}. Add the directory containing "
            "vork to PATH (for example, export PATH=\"$HOME/.local/bin:$PATH\"), "
            "or set VORK_BIN to the full path to the vork executable. "
            "The Python package contains only the SDK. Install the renderer with "
            "npm install -g @voyager-so/vork or download your platform archive from "
            "https://github.com/voyager-so/vork/releases."
        ) from None
