Edit scene.ts, then run `node scene.ts` to rewrite scene.json. Requires Node 22.18+ and npm. On older Node, upgrade (recommended), or run `node --experimental-strip-types scene.ts` instead of `node scene.ts`.
Install the SDK bundled with vork in this project:

Linux / Mac:
```sh
npm install "$HOME/.local/lib/vork/sdk"
```

Windows (PowerShell):
```powershell
npm install "$env:LOCALAPPDATA\Programs\vork\sdk"
```

For a custom installation, use its sdk directory (Unix: PREFIX/lib/vork/sdk;
Windows: PREFIX\sdk). For a source checkout, run npm ci and npm run build in packages/ts first,
then install from that directory.
Import from `@voyager-so/vork`. See `vork help sdk`.
