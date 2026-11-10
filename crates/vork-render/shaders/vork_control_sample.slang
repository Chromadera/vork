// Nearest-surface selection shared by native output/control consumers.
import vork_constants;
import vork_visibility;
public struct RasterControls { public float4 depth; public uint4 ids; public float4 normals; public float4 ambient; public float4 motion; };
// Standard four-sample raster locations, in pixel coordinates (Y down).
// Controls use either one centre sample or MSAA4.
public float2 control_sample_offset(uint sample) {
    if (sample==0u) {return float2(0.);}
    let offsets=float2[4](float2(-.125,-.375),float2(.375,-.125),float2(-.375,.125),float2(.125,.375));
    return offsets[sample-1u];
}
// Slang emits the Load coordinate of a multisampled texture as int2 on Metal, whose
// texture2d_ms::read takes uint2. Metal calls the member with the cast.
#ifdef VORK_METAL
public float load_ms(Texture2DMS<float> t,int2 p,int s) { __intrinsic_asm "$0.read(uint2($1), uint($2)).x"; }
public float4 load_ms(Texture2DMS<float4> t,int2 p,int s) { __intrinsic_asm "$0.read(uint2($1), uint($2))"; }
public uint4 load_ms(Texture2DMS<uint4> t,int2 p,int s) { __intrinsic_asm "$0.read(uint2($1), uint($2))"; }
#define MS_LOAD(t,p,s) load_ms(t,p,s)
#else
#define MS_LOAD(t,p,s) t.Load(p,s)
#endif
#define CONTROL_BODY \
var z=1.;var nearest=0;var fill=float3(0.); \
    for(var s=0;s<int(samples);s++) { \
        let d=CONTROL_LOAD(depth,p,s);if (d<z) {z=d;nearest=s;} \
        fill+=CONTROL_LOAD(ambient,p,s).rgb/float(samples); \
    } \
    /* Join a continuous crease within one pixel of depth slope, never a larger depth gap. */ \
    let transport=z; \
    if (z<1.) { \
        let selected=CONTROL_LOAD(ids,p,nearest); \
        var sum=0.;var covered=0u; \
        for(var s=0;s<int(samples);s++) { \
            let d=CONTROL_LOAD(depth,p,s); \
            let candidate=CONTROL_LOAD(ids,p,s); \
            let footprint=max(asfloat(candidate.w),asfloat(selected.w)); \
            let continuous=candidate.z==selected.z || abs(d-transport)<=footprint+4.*1.1920929e-7; \
            if (d<1. && candidate.y==selected.y && continuous) { \
                sum+=d;covered++; \
            } \
        } \
        z=sum/float(covered); \
    } \
    var normal=CONTROL_LOAD(normals,p,nearest); \
    let visibility=select(surface_visible_specular(normal.w),REG_RAY_SPECULAR,0u)|select(surface_visible_diffuse(normal.w),REG_RAY_DIFFUSE,0u); \
    normal.w=surface_encode_visibility(select(samples==1u,0.,float(nearest+1)),visibility); \
    return RasterControls(float4(z,transport,0.,0.),CONTROL_LOAD(ids,p,nearest), \
        normal,float4(fill,CONTROL_LOAD(ambient,p,nearest).w), \
        CONTROL_LOAD(motion,p,nearest));
#define CONTROL_LOAD(t,p,s) MS_LOAD(t,p,s)
public RasterControls sample_controls(int2 p,Texture2DMS<float> depth,Texture2DMS<uint4> ids,Texture2DMS<float4> normals,Texture2DMS<float4> ambient,Texture2DMS<float4> motion,uint samples) {
    CONTROL_BODY
}
#undef CONTROL_LOAD
#define CONTROL_LOAD(t,p,s) t.Load(int3(p,0))
public RasterControls sample_controls(int2 p,Texture2D<float> depth,Texture2D<uint4> ids,Texture2D<float4> normals,Texture2D<float4> ambient,Texture2D<float4> motion,uint samples) {
    CONTROL_BODY
}
#undef CONTROL_LOAD
#undef CONTROL_BODY
