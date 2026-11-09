Edit scene.py, then regenerate scene.json with the commands below. Requires Python 3.11+.
Install the SDK bundled with vork in a virtual environment:

Linux / Mac (use a Python 3.11+ executable; Ubuntu also needs its matching python3-venv package):
```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install "$HOME/.local/lib/vork/python"
python scene.py
```

Windows (PowerShell):
```powershell
py -m venv .venv
.venv\Scripts\python -m pip install "$env:LOCALAPPDATA\Programs\vork\python"
.venv\Scripts\python scene.py
```

For a custom installation, use its python directory (Unix: PREFIX/lib/vork/python;
Windows: PREFIX\python). For a source checkout use its packages/python directory.
See `vork help sdk`.
