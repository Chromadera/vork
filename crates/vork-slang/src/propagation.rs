//! Authored segment programs; numerical integration belongs to the author.
use super::*;
#[derive(Debug, Serialize, Deserialize)]
pub struct PropagationProgram {
    pub native: String,
    pub include: Vec<PathBuf>,
    pub inputs: InputLayout,
    pub dependencies: Vec<PathBuf>,
    pub warnings: String,
}
impl Compiler {
    pub fn compile_propagation(&self, abi: &Abi, source: &str, origin: &Path, tag: &str) -> Result<PropagationProgram> {
        let compiled = self.compile(abi, source, origin, tag, false, |inputs, declarations| {
            ensure!(declarations.contains("propagation"), "program requires PropagationStep propagation(PropagationContext, Inputs)");
            ensure!(inputs.fields.iter().all(|f| matches!(f.ty, Type::Float|Type::Vector(_)|Type::Int|Type::Uint|Type::Bool)), "propagation inputs must be numeric");
            let decode=inputs.slang_decode();
            Ok(format!("[shader(\"compute\")][numthreads(1,1,1)] void vork_entry() {{}}\npublic PropagationStep propagation_dispatch(float3 position,float3 direction,float time,float previous_distance,uint base) {{{decode} return propagation(PropagationContext(position,direction,time,previous_distance),inputs);}}\nexport void vork_propagation_{tag}(PropagationContext ctx,uint base,out float4 r[2]) {{let s=propagation_dispatch(ctx.position,ctx.direction,ctx.time,ctx.previous_distance,base);r[0]=float4(s.direction,s.distance);r[1]=float4(s.radiance,float(s.absorbed));}}"))
        })?;
        Ok(PropagationProgram {native:compiled.native.replace("export void","public void"), include:include_paths(origin), inputs:compiled.inputs, dependencies:compiled.output.dependencies, warnings:compiled.output.diagnostics})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires pinned Slang"]
    fn authored_step_compiles() -> Result<()> {
        let abi=Abi {aov_count:0,context:Abi::spec_context(),closure:openpbr_fields()};
        let source="struct Inputs {}; PropagationStep propagation(PropagationContext c,Inputs i) {PropagationStep s;s.direction=c.direction;s.distance=1.;return s;}";
        Compiler::discover().compile_propagation(&abi,source,Path::new("straight.slang"),"straight")?;
        Ok(())
    }
}
