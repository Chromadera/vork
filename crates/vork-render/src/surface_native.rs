//! Built-in surface entry families. Capabilities specialize in SPIR-V; only
//! resource interfaces (ray bindings and probes) select a shipped module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceKernel { Evaluate, Indirect, Occupancy, Gi, Reflections, Probes, Directional }
#[allow(dead_code)] // Entry generation is shared with build_slang.rs.
impl SurfaceKernel {
    pub const ALL: [Self;7] = [Self::Evaluate,Self::Indirect,Self::Occupancy,Self::Gi,Self::Reflections,Self::Probes,Self::Directional];
    pub fn name(self) -> &'static str {
        match self {Self::Evaluate=>"evaluate",Self::Indirect=>"indirect",Self::Occupancy=>"occupancy",Self::Gi=>"gi",Self::Reflections=>"reflections",Self::Probes=>"probes",Self::Directional=>"directional"}
    }
    pub fn camera(self) -> bool {matches!(self,Self::Evaluate|Self::Indirect|Self::Occupancy)}
    pub fn source(self) -> String {
        let (module,threads,parameters,call)=match self {
            Self::Evaluate=>("decoupled","64,1,1","uint3 gid:SV_DispatchThreadID","evaluate_material<Key,NoPrograms,true,NoPrograms>(gid);"),
            Self::Indirect=>("decoupled","REG_CAMERA_INDIRECT_GROUP,1,1","uint3 gid:SV_DispatchThreadID","indirect_material<Key,NoPrograms>(gid);"),
            Self::Occupancy=>("decoupled","8,8,1","uint3 gid:SV_DispatchThreadID","prepare_refraction_occupancy<Key,NoPrograms>(gid);"),
            Self::Gi=>("gi_ray","REG_COMPUTE_TILE,REG_COMPUTE_TILE,1","uint3 gid:SV_DispatchThreadID","gi_pixel<Key,NoPrograms>(gid);"),
            Self::Reflections=>("reflections_hits","REG_COMPUTE_TILE*REG_COMPUTE_TILE,1,1","uint3 gid:SV_DispatchThreadID","reflections_pixel<Key,NoPrograms>(gid);"),
            Self::Probes=>("probe_update","REG_PROBE_GROUP,1,1","uint3 group:SV_GroupID,uint thread:SV_GroupIndex","probe_trace<Key,NoPrograms>(group,thread);"),
            Self::Directional=>("probe_update","REG_PROBE_GROUP,1,1","uint3 group:SV_GroupID,uint thread:SV_GroupIndex","probe_directional_trace<Key,NoPrograms>(group,thread);"),
        };
        format!("import vork_constants;import frame;import surface;import {module};\n[shader(\"compute\")][numthreads({threads})] void surface_main({parameters}) {{{call}}}\n")
    }
}
