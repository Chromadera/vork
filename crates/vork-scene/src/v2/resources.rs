use super::*;
use anyhow::Context;
impl Scene {
    /// Resolve file-backed scene resources before writing a relocatable view.
    /// Inline source keeps its virtual filename so imports retain their base.
    pub fn make_resource_paths_absolute(&mut self, base:&Path) -> Result<()> {
        let base=std::path::absolute(base)?;
        fn absolute(file:&mut String,base:&Path) -> Result<()> {
            let path=Path::new(file);
            if path.is_relative() {*file=base.join(path).to_str().context("expanded resource path is not UTF-8")?.to_owned();}
            Ok(())
        }
        macro_rules! relocate {
            (string, $access:expr) => {
                for file in $access { absolute(file, &base)?; }
            };
            (json, $access:expr) => {
                for slot in $access {
                    if let Some(file) = slot.as_str() {
                        let mut file = file.to_owned();
                        absolute(&mut file, &base)?;
                        *slot = file.into();
                    }
                }
            };
        }
        super::inline::resource_files!(relocate, self);
        Ok(())
    }
}
