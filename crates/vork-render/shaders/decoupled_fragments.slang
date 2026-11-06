// Fragment-list storage and masked receiver acceptance, shared by table and
// generated program visibility entries. This module declares no shader entries.
import frame;
import vork_constants;
import stream;
import material;
import surface;
import decoupled_slots;
import transparent_opacity;
import implicit_scene;
import camera_ray;
import implicit_camera;
import hit;
import variant;

[[vk::binding(0,2)]] public RWStructuredBuffer<uint> fragments;
public uint field_descriptor(uint object) {
    return fragment_draw_table[(fragment_object_draws[object]&0x7fffffffu)*REG_RAY_SLOT_WORDS+1u];
}
// Exit faces retain their projected silhouette with synthetic near-plane
// depth (stream.slang), including when the camera is inside the domain.
public FieldHit camera_field(uint object,uint sample,float4 position,bool front,uint mirrored) {
    if(front!=(mirrored!=0u)) discard;
    float3 origin,direction;
    let size=uint2(fragments[3],fragments[4]);
    let offset=asfloat(fragment_camera_samples[sample*2u+3u].zw);
    camera_ray(sample_position(uint2(position.xy),offset),size,camera_sample_lens(fragment_camera_samples,sample),origin,direction);
    let hit=field_intersection<FragmentGeometry>(field_descriptor(object),object,origin,direction,camera_near_distance(origin,direction),camera_far_distance(origin,direction));
    if(!hit.found) discard;
    return hit;
}
public float field_depth(uint object,FieldHit hit) {
    let world=mul(float4(hit.local,1.),object_model(object));
    let clip=mul(world,frame.vp);
    let depth=clip.z/clip.w;
    if(depth<0. || depth>1.) discard;
    return depth;
}
public struct VisibilityOutput {public uint2 key:SV_Target0;public float depth:SV_Depth;};

#ifdef VORK_VOLUME_VISIBILITY
// SAMPLE_STORAGE
public void volume_depth(uint sample,float4 position,uint capacity) {
    let w=fragments[3];let h=fragments[4];let pixel=uint2(position.xy);
    // Atomic min of positive linear depth, encoded with complement bits so a
    // zero-initialized word means no fragment. Discarded fragments never record.
    let near=frame.params.x;let far=frame.params.y;
    let fraction=select(frame.flags.x>.5,(near+position.z*(far-near))/far,1./(position.w*far));
    volume_sample_record_depth(sample*w*h+pixel.y*w+pixel.x,fraction,capacity);
}
#else
public void volume_depth(uint sample,float4 position,uint capacity) {}
#endif

public void opaque_depth(uint sample,float4 position) {
    if(fragments[2]==0u || frame.field_geometry.z==0u) return;
    let list=FragmentLayout(fragments[2],fragments[3],fragments[4]);
    InterlockedMax(fragments[list.depth(sample,uint2(position.xy),max(frame.sampling.w,1u))],~asuint(position.z));
}
public VisibilityOutput masked_visibility<P:IProgram>(Varying v, bool front, uint primitive) {
    let descriptor=field_descriptor(v.object);
    if(descriptor!=0xffffffffu) {
        let mirrored=dot(cross(object_model(v.object)[0].xyz,object_model(v.object)[1].xyz),object_model(v.object)[2].xyz)<0.;
        if(front!=mirrored) discard;
        float3 origin,direction;let size=uint2(fragments[3],fragments[4]);
        let offset=asfloat(fragment_camera_samples[v.sample*2u+3u].zw);
        camera_ray(sample_position(uint2(v.position.xy),offset),size,camera_sample_lens(fragment_camera_samples,v.sample),origin,direction);
        let c=visibility_coverage(v.sample,frame.sampling.w,frame.temporal.xy);
        FieldHit hit;HitSurface point;uint crossing;
        if(!accepted_camera_field<Unspecialized,P>(descriptor,v.object,origin,direction,size,v.position,c.x,c.y,true,false,hit,point,crossing)) discard;
        opaque_depth(v.sample,point.v.position);volume_depth(v.sample,point.v.position,v.medium_capacity);
        return VisibilityOutput(visibility_key(v.object,~crossing),point.v.position.z);
    }
    v.primitive=primitive;
    let c=visibility_coverage(v.sample,frame.sampling.w,frame.temporal.xy);
    if (P.coverage ? !program_covers<P>(v,front,c.x,c.y) : !masked_covers(v,front,c.x,c.y)) {discard;}
    opaque_depth(v.sample,v.position);volume_depth(v.sample,v.position,v.medium_capacity);return VisibilityOutput(visibility_key(v.object,primitive),v.position.z);
}
