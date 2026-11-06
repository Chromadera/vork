#ifndef VORK_MEDIUM_BOUNDARIES
#define VORK_MEDIUM_BOUNDARIES 0
#endif
#ifndef VORK_VOLUME_SHADOWS
#define VORK_VOLUME_SHADOWS 0
#endif
import vork_medium_visibility;
import vork_medium_visibility_direct;
import vork_medium_visibility_grid;
import vork_prelude;
import vork_constants;
import vork_light_falloff;
import vork_medium;
import vork_sampling;
internal extern Frame medium_frame();
internal extern Light medium_light(uint index);
internal extern float4 medium_visibility_read(uint index);
internal extern uint medium_visibility_sample();
internal extern void medium_visibility_write(uint index,float4 value);
// A node supports interpolation in both neighbouring intervals. Cull only
// beyond that complete support; testing its centre would erase narrow cones.
bool medium_visibility_reaches(uint3 cell,Light light,float3 low,float3 high) {
    if((light.shadow_offsets[REG_LIGHT_CONTRIBUTION_WORD]&REG_RAY_VOLUME)==0u || all(light.radiance.rgb==float3(0.))) {return false;}
    if(light.position.w==0. || light.projection.y<=0.) {return true;}
    if(any(cell==uint3(0u)) || any(cell==uint3(REG_MEDIUM_VISIBILITY_SIDE-1u))) {return true;}
    let closest=clamp(light.position.xyz,low,high);
    return length(light.position.xyz-closest)<light.projection.y;
}
public void medium_visibility_light_world(uint3 cell) {
    let side=REG_MEDIUM_VISIBILITY_SIDE;if(any(cell>=uint3(side))) {return;}
    let f=medium_frame();
#if VORK_MEDIUM_BOUNDARIES == 0
    // A subtree capture can hide the world while sharing its installed media.
    // No source reader exists when there are no bounded domains either.
    if((f.medium.w&1u)==0u) {return;}
#endif
    let jitter_xy=sample_2d(uint2(cell.x+side*cell.z,cell.y),uint(f.temporal.x),uint(f.temporal.y),REG_SAMPLING_STREAM_VOLUME)-.5;
    let jitter_z=sample_2d(uint2(cell.x+side*(cell.z+side),cell.y),uint(f.temporal.x),uint(f.temporal.y),REG_SAMPLING_STREAM_VOLUME).y-.5;
    let p=medium_visibility_position(float3(cell)+float3(jitter_xy,jitter_z));
    let low=medium_visibility_position(max(float3(cell)-1.,float3(0.)));
    let high=medium_visibility_position(min(float3(cell)+1.,float3(side-1u)));
    for(uint i=0u;i<uint(frame_light_count(f));i++) {
        let light=medium_light(i);
        if((light.shadow_offsets[REG_LIGHT_CONTRIBUTION_WORD]&REG_RAY_VOLUME)==0u) {continue;}
        var value=float3(0.);
        if(medium_visibility_reaches(cell,light,low,high)) {
            // The lattice stores visibility at a point, so its source footprint
            // is zero. Retain the complete shared connection estimator (including
            // its emitter/shadow sampling), as the camera's point-footprint path
            // does. Reconstruction supplies the spatial interpolation.
            value=medium_direct_visibility(p,i);
        }
        let address=medium_visibility_address(cell,medium_visibility_slot(i));
        let sample=medium_visibility_sample();
        if(sample>0u) {value=lerp(medium_visibility_read(address).rgb,value,1./float(sample+1u));}
        medium_visibility_write(address,float4(value,0.));
    }
}
