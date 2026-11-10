Edit scene.py, then regenerate scene.json with the commands below. Requires Python 3.11+.
Python authoring also needs the Python SDK; create-vork installs the npm renderer/TypeScript SDK.
Download the matching vork {VERSION} release archive from
https://github.com/voyager-so/vork/releases and extract it. Its python directory
contains the SDK; replace the example archive path below with that directory.
For a matching source checkout, use its packages/python directory instead.
Use a virtual environment. VORK_BIN selects the npm renderer for Python's std()
and render() helpers; it avoids requiring a separate global vork installation.

Linux / Mac (Ubuntu also needs the matching python3-venv package):
```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install "/path/to/extracted-vork/python"
export VORK_BIN="$(node -p "require.resolve('@voyager-so/vork-'+process.platform+'-'+process.arch+'/runtime/vork')")"
python scene.py
```

Windows (PowerShell):
```powershell
py -m venv .venv
.venv\Scripts\python -m pip install "C:\path\to\extracted-vork\python"
$env:VORK_BIN = node -p "require.resolve('@voyager-so/vork-windows-x64/runtime/vork.exe')"
.venv\Scripts\python scene.py
```

Keep the SDK and renderer versions matched. See `npx vork help sdk`.
