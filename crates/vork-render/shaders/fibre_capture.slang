// Light-space neighbour counts. The evaluated closure decides
// whether a curve participates; diffuse curves retain ordinary surface lighting.
import vork_constants;
__exported import frame;
__exported import stream;
__exported import surface;
import variant;
import material;
[[vk::binding(0,2)]] Texture2D<float> fibre_first;
[[vk::binding(1,2)]] Texture2D<float> fibre_last;

float3 fibre_light_view(float3 world) {return frame.eye.w==0. ? -normalize(frame.eye.xyz) : normalize(frame.eye.xyz-world);}
Surface fibre_surface<P:IProgram>(Varying v,bool front) {
    let entering=material_front(v,front);
    if (!entering) {discard;}
    let view=fibre_light_view(v.world);
    let k=raster_coordinates(v,entering,view);
    let s=surface_at<Unspecialized,P>(v,entering,view,k);
    if (!s.is_hair || s.discarded) {discard;}
    return s;
}
public void fibre_depth<P:IProgram>(Varying v,bool front) {fibre_surface<P>(v,front);}
float fibre_metric(float z) {
    let near=frame.params.x;let far=frame.params.y;
    return lerp(near,far,z);
}
// XY is perspective for local lights; depth is linear in the selected
// cube face's forward coordinate, including support arbitrarily near the source.
public float fibre_capture_depth(Varying v) {
    if (frame.eye.w==0.) {return v.position.z;}
    let delta=abs(v.world-frame.eye.xyz);
    let depth=max(delta.x,max(delta.y,delta.z))/frame.params.y;
    if (depth<=0. || depth>=1.) {discard;}
    return depth;
}
public float4 fibre_accumulate<P:IProgram>(Varying v,bool front) {
    fibre_surface<P>(v,front);let pixel=int2(v.position.xy);
    let near=fibre_metric(fibre_first.Load(int3(pixel,0)));
    let far=fibre_metric(fibre_last.Load(int3(pixel,0)));
    let depth=fibre_metric(fibre_capture_depth(v));
    float4 crossings;
    for (uint layer=0;layer<REG_FIBRE_LAYERS;layer++) {
        let boundary=lerp(near,far,float(layer+1u)/float(REG_FIBRE_LAYERS));
        crossings[layer]=depth<=boundary ? 1. : 0.;
    }
    return crossings;
}
