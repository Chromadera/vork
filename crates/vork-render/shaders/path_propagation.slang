#ifdef VORK_PATH_CONTEXT
// One authored segment, clipped at the box. No numerical physics lives here.
#ifdef VORK_PROPAGATION
import propagation_program;
bool propagation_box(float3 p,float3 d,out float entry,out float exit) {
    entry=0.;exit=REG_FP32_MAX;
    for(uint axis=0u;axis<3u;axis++) {
        if(d[axis]==0.) {
            if(p[axis]<propagation_lower()[axis] || p[axis]>propagation_upper()[axis]) return false;
        } else {
            let a=(propagation_lower()[axis]-p[axis])/d[axis];
            let b=(propagation_upper()[axis]-p[axis])/d[axis];
            entry=max(entry,min(a,b));exit=min(exit,max(a,b));
        }
    }
    return exit>entry;
}
#endif
// radiance is applied at the segment's start, including when a hit truncates it.
// Once the box is left, the remainder of this free flight stays straight.
bool propagation_segment(float3 p,inout float3 d,inout bool escaped,inout float previous_distance,out float distance,out float3 radiance) {
    distance=REG_FP32_MAX;radiance=float3(1.);
#ifdef VORK_PROPAGATION
    if(escaped) return true;
    float entry,exit;
    if(!propagation_box(p,d,entry,exit)) {escaped=true;return true;}
    let tolerance=REG_FP32_GAMMA7*max(1.,max(abs(p.x),max(abs(p.y),abs(p.z))));
    if(entry>tolerance) {distance=entry;return true;}
    let s=propagation_dispatch(clamp(p,propagation_lower(),propagation_upper()),d,frame.scene.x,previous_distance,path_settings.propagation.x);
    if(s.absorbed) return false;
    if(!all(isfinite(s.direction)) || dot(s.direction,s.direction)<=0. || !isfinite(s.distance) || s.distance<=0. || !all(isfinite(s.radiance)) || any(s.radiance<float3(0.))) {
        path_boundary_error(REG_PATH_PROPAGATION_ERROR,1u);return false;
    }
    d=normalize(s.direction);radiance=s.radiance;
    // The program may turn outward at the boundary.
    if(!propagation_box(p,d,entry,exit)) {escaped=true;return true;}
    distance=min(s.distance,exit);escaped=s.distance>=exit;
    previous_distance=distance;
    if(all(p+d*distance==p)) {path_boundary_error(REG_PATH_PROPAGATION_ERROR,2u);return false;}
#endif
    return true;
}
struct SubsurfaceSegments : ISubsurfaceSegments {
    static bool segment(float3 p,inout float3 d,inout bool escaped,inout float previous_distance,out float limit,out float3 transfer) {
        return propagation_segment(p,d,escaped,previous_distance,limit,transfer);
    }
    static bool work(uint count) {
#ifdef VORK_PROPAGATION
        if(count>=REG_PROPAGATION_WORK_LIMIT) {path_boundary_error(REG_PATH_PROPAGATION_ERROR,3u);return false;}
#endif
        return true;
    }
};

#endif
