//! Display stage: scene-linear colour to display-encoded RGB, sharing reflection,
//! diagnostics and the cache ABI. The renderer evaluates it per output pixel.
use super::*;
#[derive(Debug, Serialize, Deserialize)]
pub struct DisplayProgram {
    pub wgsl: String,
    pub entry: String,
    pub inputs: InputLayout,
    pub dependencies: Vec<PathBuf>,
    pub warnings: String,
}
impl Compiler {
    pub fn compile_display(
        &self,
        abi: &Abi,
        source: &str,
        origin: &Path,
        tag: &str,
    ) -> Result<DisplayProgram> {
        let compiled=self.compile(abi,source,origin,tag,false,|inputs,declarations| {
            ensure!(declarations.contains("display"),"program requires float3 display(DisplayContext ctx, Inputs inputs)");
            ensure!(inputs.fields.iter().all(|f|!matches!(f.ty,Type::Geometry | Type::Attribute(_)|Type::Aov|Type::FloatGrid|Type::VectorGrid)),"display program inputs must be numeric or Image");
            let decode=inputs.slang_decode();
            Ok(format!("[shader(\"compute\")][numthreads(1,1,1)] void vork_entry() {{}}\nexport void vork_display_{tag}(float3 color,uint2 pixel,uint seed,uint bit_depth,uint base,out float4 r[1]) {{\n{decode}\nr[0]=float4(display(DisplayContext(color,pixel,seed,bit_depth),inputs),1);\n}}\n"))
        })?;
        let entry = format!("vork_display_{tag}_0_{tag}");
        ensure!(
            compiled.output.wgsl.contains(&format!("fn {entry}(")),
            "Slang did not emit {entry}"
        );
        Ok(DisplayProgram {
            wgsl: compiled.output.wgsl,
            entry,
            inputs: compiled.inputs,
            dependencies: compiled.output.dependencies,
            warnings: compiled.output.diagnostics,
        })
    }
}
