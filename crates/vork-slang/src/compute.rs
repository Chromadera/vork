//! Standalone compute entries use the same compiler, depfile and WGSL validation.
use super::*;
#[derive(Debug, Serialize, Deserialize)]
pub struct ComputeProgram {
    pub wgsl: String,
    pub inputs: InputLayout,
    pub dependencies: Vec<PathBuf>,
    pub warnings: String,
}
impl Compiler {
    /// `bindings` supplies install-resolved typed accessors before the authored module.
    /// The module defines `void compute(ComputeContext ctx, Inputs inputs)`.
    pub fn compile_compute(
        &self,
        source: &str,
        origin: &Path,
        bindings: &str,
        workgroup: u32,
        threads: [u32; 3],
    ) -> Result<ComputeProgram> {
        self.version()?;
        let work = Workspace::new()?;
        // Standard modules share the canonical context and closure in every stage.
        write_modules(
            &work,
            &Abi { aov_count: 0,
                context: Abi::spec_context(),
                closure: openpbr_fields(),
            },
            source,
        )?;
        let [x, y, z] = threads;
        let mut includes = vec![work.0.clone()];
        includes.extend(include_paths(origin));
        let program = format!(
            "import vork;\n{bindings}\n{}",
            program_text(source, origin)
        );
        let probe = format!(
            "{program}ConstantBuffer<Inputs> vork_inputs; [shader(\"compute\")][numthreads(1,1,1)] void vork_entry() {{}}"
        );
        let reflected = self.run(&work, &probe, &includes, true)?;
        let mut inputs = InputLayout::reflect(&reflected.reflection)?;
        enums::reflect(&work.0, &mut inputs)?;
        ensure!(
            inputs.fields.iter().all(|f| !matches!(
                f.ty,
                Type::Image | Type::Aov | Type::FloatGrid | Type::VectorGrid
            )),
            "compute inputs must be numeric"
        );
        let decode = inputs.slang_decode();
        let source = format!(
            "{program}\n[shader(\"compute\")][numthreads({workgroup},1,1)] void vork_entry(uint3 id:SV_DispatchThreadID) {{ if(id.x >= {x}u-vork_offset.value.x || id.y >= {y}u-vork_offset.value.y || id.z >= {z}u-vork_offset.value.z) return; id+=vork_offset.value.xyz; uint base=0; {decode} compute(ComputeContext(id, vork_tick.frame, vork_tick.time, vork_tick.dt, vork_tick.reserved), inputs); }}"
        );
        let mut output = self.run(&work, &source, &includes, false)?;
        validate(
            &output.wgsl,
            &Abi { aov_count: 0,
                context: Abi::spec_context(),
                closure: openpbr_fields(),
            },
        )?;
        output.dependencies.retain(|p| !p.starts_with(&work.0));
        Ok(ComputeProgram {
            wgsl: output.wgsl,
            inputs,
            dependencies: output.dependencies,
            warnings: output.diagnostics,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires pinned Slang"]
    fn compute_reflects_numeric_defaults_and_enum_inputs() -> Result<()> {
        let program = Compiler::discover().compile_compute(
            "enum Mode {Add=7,Multiply=11}; struct Inputs {uint amount=3; Mode mode=Mode::Add;}; void compute(ComputeContext ctx, Inputs inputs) { result_store(ctx.id.x, inputs.mode==Mode::Add ? inputs.amount : 2*inputs.amount); }",
            Path::new("compute-inputs.slang"),
            "struct Tick {uint frame;float time;float dt;uint reserved;}; [[vk::binding(0,0)]] ConstantBuffer<Tick> vork_tick; struct Offset { uint4 value; }; [[vk::binding(1,0)]] ConstantBuffer<Offset> vork_offset; [[vk::binding(2,0)]] RWStructuredBuffer<uint> result; void result_store(uint i,uint v){result[i]=v;}",
            64,[1,1,1])?;
        assert_eq!(program.inputs.fields.len(), 2);
        assert_eq!(
            program.inputs.fields[1].enumeration.as_ref().unwrap().cases,
            ["Add", "Multiply"]
        );
        assert!(program.wgsl.contains("vork_material_word"));
        Ok(())
    }
    #[test]
    #[ignore = "requires pinned Slang; compile in code-only phase"]
    fn compute_imports_bundled_library_modules() -> Result<()> {
        // Fluid includes require caller-owned buffers and callbacks; covered below.
        for module in library_modules().filter(|module| !module.material && !module.name.starts_with("fluid/")) {
            let source = format!(
                "import {}; {} void compute(ComputeContext ctx, Inputs inputs) {{ uint3 id=ctx.id; uint frame=ctx.frame; float time=ctx.time; float dt=ctx.dt; }}",
                module.name.replace('/', "."),
                if module.source.contains("struct Inputs") {
                    ""
                } else {
                    "struct Inputs {};"
                }
            );
            let program=Compiler::discover().compile_compute(
            &source,
            Path::new("compute-import.slang"),
            "struct Tick {uint frame;float time;float dt;uint reserved;}; [[vk::binding(0,0)]] ConstantBuffer<Tick> vork_tick; struct Offset { uint4 value; }; [[vk::binding(1,0)]] ConstantBuffer<Offset> vork_offset;",
            64,[1,1,1],
        )?;
            ensure!(
                program.dependencies.is_empty(),
                "embedded module workspaces must not enter dependency paths"
            );
        }
        Ok(())
    }
    #[test]
    #[ignore = "requires pinned Slang"]
    fn compute_includes_fluid_templates() -> Result<()> {
        let scene:serde_json::Value=serde_json::from_str(include_str!("../../../examples/water/scene.json"))?;
        let mut bindings=String::from("struct Tick {uint frame;float time;float dt;uint reserved;}; [[vk::binding(0,0)]] ConstantBuffer<Tick> vork_tick; struct Offset {uint4 value;}; [[vk::binding(1,0)]] ConstantBuffer<Offset> vork_offset;");
        for (name,b) in scene["buffers"].as_object().unwrap() {
            let ty=match b["type"].as_str().unwrap() {"f32"=>"float","u32"=>"uint","vec3"=>"float3","vec4"=>"float4",_=>unreachable!()};
            let dims=b["dims"].as_array().map(|d|d.iter().map(|v|v.as_u64().unwrap()).collect::<Vec<_>>()).unwrap_or_else(||vec![b["data"].as_array().unwrap().len() as u64,1,1]);
            bindings+=&format!("static const uint {name}_count={}u; static const uint3 {name}_dims=uint3({},{},{}); {ty} {name}_load(uint i) {{return {ty}(0);}} void {name}_store(uint i,{ty} v) {{}}",dims.iter().product::<u64>(),dims[0],dims[1],dims[2]);
            if ty=="uint" {bindings+=&format!("uint {name}_atomic_min(uint i,uint v) {{return 0xffffffffu;}}");}
        }
        for name in ["init","fit","white_step"] {
            let result=Compiler::discover().compile_compute(scene["programs"][name]["source"].as_str().unwrap(),Path::new("fluid-include.slang"),&bindings,64,[1,1,1])?;
            ensure!(result.dependencies.is_empty(),"bundled fluid includes must not depend on checkout paths");
        }
        Ok(())
    }

}
