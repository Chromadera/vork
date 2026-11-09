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
        let scene = vork_scene::STARTER_SCENE;
        let mut files = vec![("scene.json", scene.to_owned())];
        let author = match self.language {
            Language::Json => "Edit scene.json, then render again.\n".to_owned(),
            Language::Typescript => {
                files.push(("package.json", "{\n  \"private\": true,\n  \"type\": \"module\"\n}\n".to_owned()));
                let mut value: serde_json::Value = serde_json::from_str(scene)?;
                for (i, name) in ["blue", "coral", "floor"].iter().enumerate() { value["nodes"][i]["geometry"] = (*name).into(); }
                let literal = serde_json::to_string_pretty(&value)?;
                files.push(("scene.ts", format!("import {{ writeScene, sphere, box, grid, type Scene }} from '@voyager-so/vork';\n\nconst blue = sphere('blue', {{radius: 0.85, segments: 64, rings: 32}});\nconst coral = box('coral', {{size: [0.9, 0.9, 0.9]}});\nconst floor = grid('floor', {{size: [200, 200]}});\nconst scene = {{...{literal}, buffers: {{...blue.buffers, ...coral.buffers, ...floor.buffers}}, geometry: {{...blue.geometry, ...coral.geometry, ...floor.geometry}}}} satisfies Scene;\nawait writeScene(scene, 'scene.json');\n")));
                include_str!("starter-typescript.md").to_owned()
            }
            Language::Python => {
                let mut value: serde_json::Value = serde_json::from_str(scene)?;
                for (i, name) in ["blue", "coral", "floor"].iter().enumerate() { value["nodes"][i]["geometry"] = (*name).into(); }
                let literal = serde_json::to_string_pretty(&value)?;
                files.push(("scene.py", format!("from vork import std, write_scene\n\nscene = {literal}\nscene['buffers'] = {{}}\nscene['geometry'] = {{}}\nfor shape, options in [('sphere', {{'name': 'blue', 'radius': 0.85, 'segments': 64, 'rings': 32}}), ('box', {{'name': 'coral', 'size': [0.9, 0.9, 0.9]}}), ('grid', {{'name': 'floor', 'size': [200, 200]}})]:\n    fragment = std(shape, options)\n    scene['buffers'].update(fragment['buffers'])\n    scene['geometry'].update(fragment['geometry'])\nwrite_scene(scene, 'scene.json')\n")));
                include_str!("starter-python.md").to_owned()
            }
        };
        files.push(("README.md", format!("# Your first vork scene\n\n{author}\n```sh\nvork render scene.json --out render.png\nvork help\n```\n\nOpen render.png. Try changing the colours, camera target or light positions.\nThe default `std/final` profile is warmed by setup; use `--profile std/draft` for quicker previews. No fetched assets are needed.\n")));
        fs::create_dir_all(&self.dir).with_context(|| format!("create {}", self.dir.display()))?;
        for (name, _) in &files {
            if self.dir.join(name).symlink_metadata().is_ok() {
                return Err(anyhow::anyhow!("{} already exists; choose another directory (no files overwritten)", self.dir.join(name).display()).context(vork_render::failure::FailureKind::Usage));
            }
        }
        for (name, content) in files {
            fs::OpenOptions::new().write(true).create_new(true).open(self.dir.join(name))?.write_all(content.as_bytes())?;
        }
        output!("Created {}\nNext: enter that directory, then run:\n  vork render scene.json --out render.png\n  vork help\nRead README.md for authoring steps.", self.dir.display());
        if matches!(self.language, Language::Typescript) {
            output!("TypeScript requires Node 22.18+ and npm. On older Node, upgrade (recommended), or run node --experimental-strip-types scene.ts.");
        }
        Ok(())
    }
}
