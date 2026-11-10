//! Standalone compute entries use the same compiler, depfile and WGSL validation.
use super::*;
#[derive(Debug, Serialize, Deserialize)]
pub struct ComputeProgram {
    pub wgsl: String,
    pub slang: String,
    pub inputs: InputLayout,
    pub dependencies: Vec<PathBuf>,
    pub warnings: String,
    pub queries: Vec<QueryFilter>,
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
        let queries=if effects::calls_trace(&output.wgsl,"vork_entry",&Abi {aov_count:0,context:Abi::spec_context(),closure:openpbr_fields()})? {
            effects::compute_query_filters(&output.wgsl,&Abi {aov_count:0,context:Abi::spec_context(),closure:openpbr_fields()})?
        } else {Vec::new()};
        output.dependencies.retain(|p| !p.starts_with(&work.0));
        Ok(ComputeProgram {
            queries,
            slang: source,
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
    fn compute_query_filters_follow_arguments_and_reject_unbounded_work() -> Result<()> {
        let compiler=Compiler::discover();
        let header="struct Tick {uint frame;float time;float dt;uint reserved;}; [[vk::binding(0,0)]] ConstantBuffer<Tick> vork_tick;struct Offset {uint4 value;};[[vk::binding(1,0)]] ConstantBuffer<Offset> vork_offset;[[vk::binding(2,0)]] RWStructuredBuffer<Atomic<uint>> result;void result_store(uint i,float v){result[i].store(asuint(v));}uint result_min(uint i,uint v){return result[i].min(v);}";
        for (index,body,expected) in [
            (0,"result_store(ctx.id.x,1.);",vec![]),
            (1,"result_store(ctx.id.x,probe(ctx.id,2u,1u));",vec![QueryFilter {mask:2,object:Some(1)}]),
            (2,"result_store(ctx.id.x,probe(ctx.id,255u,inputs.target));",vec![QueryFilter {mask:255,object:None}]),
            (3,"for(uint k=0u;k<3u;k++) {result_store(ctx.id.x+k,probe(ctx.id,2u,k+1u));}",vec![QueryFilter {mask:2,object:Some(1)},QueryFilter {mask:2,object:Some(2)},QueryFilter {mask:2,object:Some(3)}]),
            (4,"result_min(ctx.id.x,4u);result_store(ctx.id.x,probe(ctx.id,2u,1u));",vec![QueryFilter {mask:2,object:Some(1)}]),
        ] {
            let source=format!("struct Inputs {{uint target=1u;}};float probe(uint3 id,uint mask,uint object) {{return trace(float3(id),float3(0.,-1.,0.),0.,10.,mask,object).distance;}}void compute(ComputeContext ctx,Inputs inputs) {{{body}}}");
            let program=compiler.compile_compute(&source,Path::new(&format!("compute-query-{index}.slang")),header,64,[3,1,1])?;
            assert_eq!(program.queries,expected,"case {index}");
        }
        let source="struct Inputs {bool again=false;};void compute(ComputeContext ctx,Inputs i) {while(i.again){result_store(ctx.id.x,trace(float3(0.),float3(0.,-1.,0.),0.,1.).distance);}}";
        assert!(format!("{:#}",compiler.compile_compute(source,Path::new("compute-query-loop.slang"),header,64,[1,1,1]).unwrap_err()).contains("provably finite"));
        Ok(())
    }
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
        // Simulation templates require caller-owned buffers/configuration; covered below.
        for module in library_modules().filter(|module| !module.material && !module.name.starts_with("fluid/") && !module.name.starts_with("cloth/")) {
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
    #[ignore = "requires pinned Slang and Python SDK"]
    fn compute_includes_cloth_templates() -> Result<()> {
        let script=r#"
import json
from vork.cloth import attach
scenes=[]
for collider in [None,'body']:
    scene=dict(buffers=dict(p=dict(type='vec3',data=[[0,1,0],[1,1,0],[0,1,1]]),uv=dict(type='vec2',data=[[0,0],[-1,0],[0,1]]),bp=dict(type='vec3',data=[[-1,0,-1],[1,0,-1],[0,0,1]])),geometry=dict(cloth=dict(attributes=dict(POSITION='p',TEXCOORD_0='uv')),body=dict(attributes=dict(POSITION='bp'))),nodes=[dict(type='mesh',name='cloth',geometry='cloth'),dict(type='mesh',name='body',geometry='body')])
    attach(scene,'cloth',collider=collider,pins=[0])
    scenes.append(scene)
print(json.dumps(scenes))
"#;
        let python=if cfg!(target_os="windows") {"python"} else {"python3"};
        let output=std::process::Command::new(python).args(["-c",script])
            .env("PYTHONPATH",Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/python"))
            .output().context("generate configured cloth wrappers with the Python SDK")?;
        ensure!(output.status.success(),"cloth authoring failed: {}",String::from_utf8_lossy(&output.stderr));
        let scenes:Vec<serde_json::Value>=serde_json::from_slice(&output.stdout)?;
        for scene in scenes {
            for program in scene["programs"].as_object().unwrap().values() {
                let mut bindings=String::from("struct Tick {uint frame;float time;float dt;uint reserved;}; [[vk::binding(0,0)]] ConstantBuffer<Tick> vork_tick; struct Offset {uint4 value;}; [[vk::binding(1,0)]] ConstantBuffer<Offset> vork_offset;");
                // The renderer supplies these evaluated-geometry accessors; this
                // code-only test checks includes/types, while L4 tests exercise sampling.
                bindings+="float3 geometry_POSITION_load(uint i){return float3(0);} float3 geometry_POSITION_load(Geometry g,uint i){return float3(0);} float3 geometry_to_world(Geometry g,float3 p){return p;}";
                for (alias,target) in program["bindings"].as_object().unwrap() {
                    let buffer=&scene["buffers"][target.as_str().unwrap()];
                    let ty=match buffer["type"].as_str().unwrap() {"f32"=>"float","u32"=>"uint","vec2"=>"float2","vec3"=>"float3","vec4"=>"float4",_=>unreachable!()};
                    bindings+=&format!("{ty} {alias}_load(uint i){{return {ty}(0);}} void {alias}_store(uint i,{ty} value){{}}");
                }
                let result=Compiler::discover().compile_compute(program["source"].as_str().unwrap(),Path::new("cloth-include.slang"),&bindings,64,[64,1,1])?;
                ensure!(result.dependencies.is_empty(),"bundled cloth includes must not depend on checkout paths");
            }
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

/// Entry resource use, including opaque query arguments, for installed compute views.
pub fn compute_used_bindings(source:&str)->Result<std::collections::BTreeSet<u32>> {
    let source=format!("{source}\n{}",crate::trace::validation_stubs());
    let module=naga::front::wgsl::parse_str(&source).map_err(|error|anyhow::anyhow!(error.emit_to_string(&source)))?;
    let info=naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::SHADER_FLOAT16_IN_FLOAT32).validate(&module).map_err(|error|anyhow::anyhow!("{error:?}"))?;
    let entry=module.entry_points.iter().position(|entry|entry.name=="vork_entry").context("compute input reflection requires vork_entry")?;
    Ok(module.global_variables.iter().filter_map(|(handle,global)|global.binding.filter(|binding|binding.group==0 && !info.get_entry_point(entry)[handle].is_empty()).map(|binding|binding.binding)).collect())
}
