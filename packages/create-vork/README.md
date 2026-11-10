# create-vork

Create a vork scene and install its renderer and SDK:

```sh
npm create vork@latest my-shot
cd my-shot
npx vork render scene.json --out render.png
npx vork help
```

Use `npm create vork@latest my-shot -- --language typescript` for editable
TypeScript source, or `--language python` for Python source. Each project includes
scene.json and a README with authoring instructions. Existing project files are
preserved; choose a new directory if initialization reports a conflict.

Requires Node 22.18+ and npm, plus a supported GPU and drivers. Older Node emits
EBADENGINE warnings; upgrade before installing. TypeScript source on older Node
versions that support type stripping needs `node --experimental-strip-types scene.ts`.
The TypeScript SDK is `@voyager-so/vork`; create-vork installs it automatically.
Keep npm optional dependencies enabled for the native renderer. Python authoring
also requires the Python SDK; see the generated README.

See https://github.com/voyager-so/vork for platform requirements and examples.
