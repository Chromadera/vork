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
        let scene = include_str!("starter.json");
        let mut files = vec![("scene.json", scene.to_owned())];
        let author = match self.language {
            Language::Json => "Edit scene.json, then render again.\n".to_owned(),
            Language::Typescript => {
                files.push(("package.json", "{\n  \"private\": true,\n  \"type\": \"module\"\n}\n".to_owned()));
                let mut value: serde_json::Value = serde_json::from_str(scene)?;
                for (i, name) in ["blue", "coral", "floor"].iter().enumerate() { value["nodes"][i]["geometry"] = (*name).into(); }
                let literal = serde_json::to_string_pretty(&value)?;
                files.push(("scene.ts", format!("import {{ writeScene, sphere, box, grid, type Scene }} from '@voyager-so/vork';\n\nconst blue = sphere('blue', {{radius: 0.85, segments: 64, rings: 32}});\nconst coral = box('coral', {{size: [0.9, 0.9, 0.9]}});\nconst floor = grid('floor', {{size: [200, 200]}});\nconst scene = {{...{literal}, buffers: {{...blue.buffers, ...coral.buffers, ...floor.buffers}}, geometry: {{...blue.geometry, ...coral.geometry, ...floor.geometry}}}} satisfies Scene;\nawait writeScene(scene, 'scene.json');\n")));
                "Edit scene.ts, then run `node scene.ts` to rewrite scene.json. Requires Node 22.18+ and @voyager-so/vork installed in this project. Install the SDK bundled with vork using `npm install PREFIX/lib/vork/sdk`. Replace PREFIX with your vork installation prefix (default: ~/.local). For a source checkout use its packages/ts directory. See `vork help sdk`.\n".to_owned()
            }
            Language::Python => {
                let mut value: serde_json::Value = serde_json::from_str(scene)?;
                for (i, name) in ["blue", "coral", "floor"].iter().enumerate() { value["nodes"][i]["geometry"] = (*name).into(); }
                let literal = serde_json::to_string_pretty(&value)?;
                files.push(("scene.py", format!("from vork import std, write_scene\n\nscene = {literal}\nscene['buffers'] = {{}}\nscene['geometry'] = {{}}\nfor shape, options in [('sphere', {{'name': 'blue', 'radius': 0.85, 'segments': 64, 'rings': 32}}), ('box', {{'name': 'coral', 'size': [0.9, 0.9, 0.9]}}), ('grid', {{'name': 'floor', 'size': [200, 200]}})]:\n    fragment = std(shape, options)\n    scene['buffers'].update(fragment['buffers'])\n    scene['geometry'].update(fragment['geometry'])\nwrite_scene(scene, 'scene.json')\n")));
                "Edit scene.py, then run `python scene.py` to rewrite scene.json. Requires Python 3.11+ and the vork SDK installed in your environment. In a virtual environment, install the SDK bundled with vork using `python -m pip install PREFIX/lib/vork/python`. Replace PREFIX with your vork installation prefix (default: ~/.local). For a source checkout use its packages/python directory. See `vork help sdk`.\n".to_owned()
            }
        };
        files.push(("README.md", format!("# Your first vork scene\n\n{author}\n```sh\nvork render scene.json --profile std/draft --out render.png\nvork help\n```\n\nOpen render.png. Try changing the colours, camera target or light positions.\nUse `--profile std/final` for a cleaner image. No fetched assets are needed.\nFor an npm project use `npx vork` in place of `vork`.\n")));
        fs::create_dir_all(&self.dir).with_context(|| format!("create {}", self.dir.display()))?;
        for (name, _) in &files {
            if self.dir.join(name).symlink_metadata().is_ok() {
                return Err(anyhow::anyhow!("{} already exists; choose another directory (no files overwritten)", self.dir.join(name).display()).context(vork_render::failure::FailureKind::Usage));
            }
        }
        for (name, content) in files {
            fs::OpenOptions::new().write(true).create_new(true).open(self.dir.join(name))?.write_all(content.as_bytes())?;
        }
        output!("Created {}\nNext: enter that directory, then run:\n  vork render scene.json --profile std/draft --out render.png\n  vork help\nRead README.md for authoring steps.", self.dir.display());
        Ok(())
    }
}
