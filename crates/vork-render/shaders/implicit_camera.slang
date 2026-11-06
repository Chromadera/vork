// Camera visibility and nominal controls walk the same exact crossings, retaining
// the evaluated surface and position of the first root their coverage accepts.
import implicit_scene;
import transparent_opacity;
import variant;
import surface;
import hit;
import stream;
import frame;
import vork_constants;
import camera_ray;

bool accepted_camera_field<V:IVariant,P:IProgram>(uint descriptor,uint object,
    float3 origin,float3 direction,uint2 size,float4 position,uint index,uint count,
    bool coverage,bool nonzero,out FieldHit hit,out HitSurface point,out uint crossing) {
    point={};
    var minimum=camera_near_distance(origin,direction);
    let maximum=camera_far_distance(origin,direction);
    for(crossing=0u;;crossing++) {
        hit=field_intersection<FragmentGeometry>(descriptor,object,origin,direction,minimum,maximum);
        if(!hit.found) return false;
        point=field_surface<V,P>(object,hit,direction,float2(2.)/float2(size),Lobe(0u,0.));
        let clip=mul(float4(point.v.world,1.),frame.vp);
        point.v.position=float4(position.xy,clip.z/clip.w,1./clip.w);
        point.v.primitive=~crossing;
        if(coverage) point.s=raster_coverage(point.v,point.s,index,count);
        if(!point.s.discarded && (!nonzero || point.s.closure.geometry_opacity*objects[object].opacity.x>0.)) return true;
        minimum=hit.t+REG_FP32_GAMMA7*max(1.,abs(hit.t));
        if(minimum>maximum) return false;
    }
}
