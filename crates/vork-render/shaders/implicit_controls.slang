// Exact nominal camera geometry for depth/normal/ID and denoising guides.
import frame;
import vork_constants;
import stream;
import variant;
import surface;
import transparent_opacity;
import implicit_scene;
import camera_ray;
import implicit_camera;
import hit;
bool implicit_control_point<V:IVariant,P:IProgram>(inout Varying v,inout bool front,bool transparent,out Surface surface) {
    surface={};
    let descriptor=fragment_draw_table[(fragment_object_draws[v.object]&0x7fffffffu)*REG_RAY_SLOT_WORDS+1u];
    if(descriptor==0xffffffffu) return false;
    let model=object_model(v.object);
    let mirrored=dot(cross(model[0].xyz,model[1].xyz),model[2].xyz)<0.;
    if(front!=mirrored) discard;
    let size=uint2(round(2./frame.lens.zw));float3 origin,direction;
    camera_ray(v.position.xy,size,float2(0.),origin,direction);
    FieldHit hit;HitSurface point;uint crossing;
    if(!accepted_camera_field<V,P>(descriptor,v.object,origin,direction,size,v.position,uint(frame.temporal.x),uint(frame.temporal.y),V.has_mask || transparent,transparent,hit,point,crossing)) discard;
    var result=point.v;front=point.entering!=mirrored;
    surface=point.s;
    result.previous_clip=mul(mul(float4(hit.local,1.),object_previous_model(v.object)),frame.previous_vp);
    result.next_clip=mul(mul(float4(hit.local,1.),object_next_model(v.object)),frame.next_vp);
    v=result;return true;
}
