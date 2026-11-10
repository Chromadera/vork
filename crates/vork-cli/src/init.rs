//! One bundled scene, with optional ordinary authoring source. No network or GPU.
use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use std::{fs, io::Write, path::PathBuf};

#[derive(Clone, Copy, ValueEnum)]
pub enum Language { Json, Typescript, Python }

#[derive(Parser)]
pub struct Args {
    /// Project directory (default: current directory). Existing unrelated files are preserved.
    #[arg(default_value = ".")]
    dir: PathBuf,
    /// Also write editable TypeScript or Python source; scene.json always renders immediately.
    #[arg(long, value_enum, default_value = "json")]
    language: Language,
}
impl Args {
    pub fn run(&self) -> Result<()> {
        self.create()
    }
    fn create(&self) -> Result<()> {
        let npm = std::env::var_os("VORK_NPM_CONTEXT").is_some();
        let command = if npm { "npx vork" } else { "vork" };
        let scene = vork_scene::STARTER_SCENE;
        let mut files = vec![("scene.json", scene.to_owned())];
        let author = match self.language {
            Language::Json => "Edit scene.json, then render again.\n".to_owned(),
            Language::Typescript => {
                files.push(("package.json", "{\n  \"private\": true,\n  \"type\": \"module\"\n}\n".to_owned()));
                let value: serde_json::Value = serde_json::from_str(scene)?;
                let literal = serde_json::to_string_pretty(&value)?;
                files.push(("scene.ts", format!("import {{ writeScene, type Scene }} from '@voyager-so/vork';\n\nconst scene = {literal} satisfies Scene;\nawait writeScene(scene, 'scene.json');\n")));
                if npm { "Edit scene.ts, then run `node scene.ts` to rewrite scene.json. Requires Node 22.18+ and npm. Older Node gives EBADENGINE warnings; upgrade, or use `node --experimental-strip-types scene.ts` where supported.\nInstall the SDK with `npm install @voyager-so/vork` (create-vork installs it for you).\nImport from `@voyager-so/vork`. See `npx vork help sdk`.\n".to_owned() } else { include_str!("starter-typescript.md").to_owned() }
            }
            Language::Python => {
                let value: serde_json::Value = serde_json::from_str(scene)?;
                let literal = serde_json::to_string_pretty(&value)?;
                files.push(("scene.py", format!("import json\nfrom vork import write_scene\n\nscene = json.loads(r'''{literal}''')\nwrite_scene(scene, 'scene.json')\n")));
                if npm { include_str!("starter-python-npm.md").replace("{VERSION}", env!("CARGO_PKG_VERSION")) } else { include_str!("starter-python.md").to_owned() }
            }
        };
        files.push(("README.md", format!("# Your first vork scene\n\n{author}\n```sh\n{command} render scene.json --out render.png\n{command} help\n```\n\nOpen render.png. Try changing the colours, camera target or light positions.\nThe default `std/final` profile is warmed by setup; use `--profile std/draft` for quicker previews. For interiors and full bounce lighting, use `--profile std/path --set samples=64` to start; raise samples for delivery. See `{command} help profiles` for the speed/quality table. No fetched assets are needed.\n")));
        fs::create_dir_all(&self.dir).with_context(|| format!("create {}", self.dir.display()))?;
        for name in files.iter().map(|(name, _)| *name) {
            if self.dir.join(name).symlink_metadata().is_ok() {
                return Err(anyhow::anyhow!("{} already exists; choose another directory (no files overwritten)", self.dir.join(name).display()).context(vork_render::failure::FailureKind::Usage));
            }
        }
        for (name, content) in files {
            fs::OpenOptions::new().write(true).create_new(true).open(self.dir.join(name))?.write_all(content.as_bytes())?;
        }
        output!("Created {}\nNext: enter that directory, then run:\n  {command} render scene.json --out render.png\n  {command} help\nRead README.md for authoring steps.", self.dir.display());
        if matches!(self.language, Language::Typescript) {
            output!("TypeScript requires Node 22.18+ and npm. On older Node, upgrade (recommended), or run node --experimental-strip-types scene.ts.");
        }
        Ok(())
    }
}
