// Resolve, lens/shutter filtering and exact output controls. Native slots follow resolve_native.rs.
import radiance_terms;
import vork_constants;
import output_diagnostics;
import vork_prelude;
import vork_sampling;
import vork_visibility;
[[vk::binding(67,0)]] StructuredBuffer<uint> sampling;

uint2 dimensions(Texture2D<float4> t) { uint w,h;t.GetDimensions(w,h);return uint2(w,h); }
uint2 dimensions(Texture2D<uint4> t) { uint w,h;t.GetDimensions(w,h);return uint2(w,h); }
uint3 dimensions(Texture3D<float4> t) { uint w,h,d;t.GetDimensions(w,h,d);return uint3(w,h,d); }
uint pack_unorm(float4 v) {uint4 q=uint4(round(saturate(v)*255.));return q.x|(q.y<<8u)|(q.z<<16u)|(q.w<<24u);}

[[vk::binding(0,0)]] Texture2D<float4> color;
[[vk::binding(1,0)]] Texture2D<float4> depth;
[[vk::binding(2,0)]] Texture2D<uint4> ids;
[[vk::binding(11,0)]] StructuredBuffer<float> diffuse_response;
[[vk::binding(0,1)]] [[vk::image_format("rgba8")]] RWTexture2D<float4> output;
[[vk::binding(4,0)]] RWStructuredBuffer<uint> pixels;
[[vk::binding(47,0)]] RWStructuredBuffer<uint> nonfinite;
[[vk::binding(70,0)]] RWStructuredBuffer<uint> output_errors;
#ifdef VORK_VOLUME_BINDINGS
import frame;
#else
[[vk::binding(5,0)]] ConstantBuffer<Frame> frame;
#endif
[[vk::binding(6,0)]] Texture2D<float4> normals;
[[vk::binding(7,0)]] Texture2D<float4> ambient;
[[vk::binding(8,0)]] Texture2D<float4> ao_texture;
[[vk::binding(58,0)]] Texture2D<float4> ao_cell_receivers;
[[vk::binding(68,0)]] Texture2D<float4> ao_receivers;
[[vk::binding(69,0)]] Texture2D<float4> ao_normals;

[[vk::binding(9,0)]] Texture2D<float4> motion;
[[vk::binding(10,0)]] RWStructuredBuffer<float4> accumulation;
#if !defined(VORK_VOLUME_BINDINGS) || !defined(VORK_PROGRAM_COVERAGE)
[[vk::binding(12,0)]] SamplerState env_sampler;
#endif
// The forward BRDF's prefiltered environment remains the miss radiance.
// Split-sum response follows the split-sum F0*energy.x + F90*energy.y approximation;
// screen marching is local to vork, with no imported scene/framegraph machinery.
[[vk::binding(16,0)]] Texture2D<float4> env_specular;

[[vk::binding(41,0)]] Texture2D<float4> indirect;
import vork_control_sample;
#ifdef VORK_RESOLVE_SINGLE
[[vk::binding(51,0)]] Texture2D<float> raster_depth;
[[vk::binding(52,0)]] Texture2D<uint4> raster_ids;
[[vk::binding(53,0)]] Texture2D<float4> raster_normals;
[[vk::binding(54,0)]] Texture2D<float4> raster_ambient;
[[vk::binding(55,0)]] Texture2D<float4> raster_motion;
static const uint raster_samples=1;
#else
[[vk::binding(51,0)]] Texture2DMS<float> raster_depth;
[[vk::binding(52,0)]] Texture2DMS<uint4> raster_ids;
[[vk::binding(53,0)]] Texture2DMS<float4> raster_normals;
[[vk::binding(54,0)]] Texture2DMS<float4> raster_ambient;
[[vk::binding(55,0)]] Texture2DMS<float4> raster_motion;
static const uint raster_samples=4;
#endif
// EffectPlan::fuse_controls excludes GI, traced reflections, denoising and
// compositing. Keep that contract static so unused filters cannot inflate it.
#ifdef VORK_RESOLVE_FUSED
static const bool fused_controls=true;
#else
static const bool fused_controls=false;
#endif
static RasterControls raster_controls;
float4 control_depth(int2 p) {if(fused_controls) return raster_controls.depth;return depth.Load(int3(p,0));}
uint4 control_ids(int2 p) {if(fused_controls) return raster_controls.ids;return ids.Load(int3(p,0));}
float4 control_normals(int2 p) {if(fused_controls) return raster_controls.normals;return normals.Load(int3(p,0));}
float4 control_ambient(int2 p) {if(fused_controls) return raster_controls.ambient;return ambient.Load(int3(p,0));}
float4 control_motion(int2 p) {if(fused_controls) return raster_controls.motion;return motion.Load(int3(p,0));}
float4 hemisphere(int2 p) {
    if (fused_controls || (frame.ao.y<=0. && frame.flags.w==0.)) {return float4(0.,0.,0.,1.);}
    let receiver=ao_receivers.Load(int3(p,0));
    if (receiver.w>=1.) {return float4(0.,0.,0.,1.);}
    // Full-resolution AO is already filtered at this exact pixel.
    if (frame.ao.w==1.) {
        var incident=float3(0.);if (frame.flags.w>0.) {incident=indirect.Load(int3(p,0)).rgb;}
        return float4(incident,ao_texture.Load(int3(p,0)).x);
    }
    let coord=(float2(p)+.5)/frame.ao.w-.5;let base=int2(floor(coord));
    let position=receiver.xyz;
    let view_depth=-(mul(float4(position,1.),frame.view)).z;let n=normalize(ao_normals.Load(int3(p,0)).xyz);
    let thickness=max(REG_AO_FILTER_MIN_THICKNESS,view_depth*REG_AO_FILTER_DEPTH_SCALE);
    var sum=0.;var weights=0.;var incident=float3(0.);
    for(var y=1-REG_AO_UPSAMPLE_RADIUS;y<=REG_AO_UPSAMPLE_RADIUS;y++) {for(var x=1-REG_AO_UPSAMPLE_RADIUS;x<=REG_AO_UPSAMPLE_RADIUS;x++) {
        let q=clamp(base+int2(x,y),int2(0),int2(dimensions(ao_texture))-1);
        let tap=ao_texture.Load(int3(q,0));if (tap.y<=0.) {continue;}
        let other=ao_cell_receivers.Load(int3(q,0)).xyz;
        let other_normal=decode_octahedron(tap.zw);
        let dz=dot(other-position,n)/thickness;
        let spatial=max(float2(0.),float2(1.)-abs(float2(base+int2(x,y))-coord)/float(REG_AO_UPSAMPLE_RADIUS));
        let weight=spatial.x*spatial.y*exp(-dz*dz)*pow(max(dot(n,other_normal),0.),REG_AO_BLUR_NORMAL_POWER);
        sum+=tap.x*weight;weights+=weight;
        if (frame.flags.w>0.) {incident+=indirect.Load(int3(q,0)).rgb*weight;}
    }}
    if (weights<=REG_POSITIVE_EPSILON) {return float4(0.,0.,0.,1.);}
    return float4(incident/weights,sum/weights);
}

[[vk::binding(24,0)]] Texture2DArray<float4> reflections;
// Preserve per-pixel Monte Carlo estimates for the final denoiser. Filtering
// signed replacement terms here both moves the retained environment baseline
// across receivers and turns sampling noise into spatially coherent features.
float3 screen_reflection(int2 p) {
    if (fused_controls || frame.projection.z!=REG_REFLECTIONS_RAY || frame.sampling.w==0u) {return float3(0.);}
    var sum=float3(0.);
    for (uint c=0u;c<REG_REFLECTION_CLASSES;c++) {sum+=reflections.Load(int4(p,int(c),0)).rgb;}
    return sum;
}
// WORLD MEDIUM: medium.rs installs the volume integration entries here.
bool colour_requires_controls() {return !fused_controls;}
float3 linear_color(int2 p) {
    // Spectral matching functions have signed RGB samples. Retain the signs
    // until exposure integration; clipping each sample biases their mean.
    if (!colour_requires_controls()) {return color.Load(int3(p,0)).rgb;}
    let estimate=hemisphere(p);let visibility=estimate.a;
    // This attachment carries only the exact diffuse baseline. Glossy,
    // emission and direct light belong to independent providers.
    let fill=control_ambient(p).rgb;
    var response=float3(0.);
    // The decoupled receiver gates the estimate (hemisphere); pixels without opaque slots carry a zero response.
    if (!fused_controls && frame.flags.w>0.) {
        let at=(uint(p.y)*dimensions(color).x+uint(p.x))*3u;
        response=float3(diffuse_response[at],diffuse_response[at+1u],diffuse_response[at+2u]);
        // The shade's coverage- and transmittance-weighted response (decoupled.slang).

    }
    return compose_indirect(color.Load(int3(p,0)).rgb,fill,visibility,screen_reflection(p),response,estimate.rgb);
}
float4 scene_rgba(int2 p) {
    let alpha=select(frame.flags.z>0.,clamp(color.Load(int3(p,0)).a,0.,1.),1.);
    return float4(linear_color(p),alpha);
}
float4 shutter_color(int2 p,float z) {
    let rgba=scene_rgba(p);
    var linear=rgba.rgb;
    // Forward colour is premultiplied over transparent black. Export straight RGBA.
    var alpha=1.;
    if (frame.flags.z>0.) {
        alpha=clamp(rgba.a,0.,1.);
        linear=select(alpha>0.,linear/max(alpha,REG_POSITIVE_EPSILON),float3(0.));
    }
    return float4(linear,alpha);
}
[[vk::binding(45,0)]] StructuredBuffer<float4> post_color;
uint2 output_size() {uint w,h;output.GetDimensions(w,h);return uint2(w,h);}
void store_color(float4 associated,float3 unassociated,float alpha,uint2 pixel,uint i,uint count,uint flags) {
    if (!all(isfinite(associated)) || !all(isfinite(unassociated)) || !isfinite(alpha)) {InterlockedMax(output_errors[0],REG_RADIANCE_NONFINITE);}
    check_nonfinite(nonfinite,REG_NONFINITE_RESOLVE,associated);
    if ((flags&REG_OUTPUT_FLOAT_COLOR)!=0u) {
        for(var channel=0u;channel<4u;channel++) {
            pixels[(REG_FLOAT_COLOR_PLANE+channel)*count+i]=asuint(associated[channel]);
        }
    }
    if ((flags&(REG_OUTPUT_GPU_COLOR|REG_OUTPUT_COLOR_PLANE))!=0u) {
        let base=uint(frame.display.w);let value=float4(unassociated,alpha);
        for(uint channel=0u;channel<4u;channel++) {pixels[(base+channel)*count+i]=asuint(value[channel]);}
    }
}

void resolve_pixel(uint3 gid) {
    let size=dimensions(color);if (gid.x>=size.x || gid.y>=size.y) {return;}
    let p=int2(gid.xy);let i=gid.y*size.x+gid.x;let count=size.x*size.y;
    let full_i=gid.y*size.x+gid.x;
    let flags=uint(frame.projection.w);let final_sample=frame.temporal.x+1.>=frame.temporal.y;
    let controls=REG_OUTPUT_DEPTH_IDS|REG_OUTPUT_NORMALS|REG_OUTPUT_MOTION|REG_OUTPUT_FLOAT_DEPTH|REG_OUTPUT_FLOAT_NORMALS|REG_OUTPUT_FLOAT_MOTION;
    // Colour without post filters needs no surface guides. Output controls are
    // already published only on the final sample; acquire their inputs then.
    if(fused_controls && (colour_requires_controls() || (final_sample && (flags&controls)!=0u))) {
        raster_controls=sample_controls(p,raster_depth,raster_ids,raster_normals,raster_ambient,raster_motion,raster_samples);
    }
    if ((flags&(REG_OUTPUT_GPU_COLOR|REG_OUTPUT_COLOR_PLANE|REG_OUTPUT_FLOAT_COLOR))!=0u) {
        float3 linear;float alpha;float4 associated;
        if (!fused_controls && frame.post.x>0.) {
            if (!final_sample) {return;}
            associated=post_color[full_i];
            if (frame.post.x==2.) {associated/=frame.temporal.y;}
            alpha=associated.a;
            linear=associated.rgb;
            if (alpha>0.) {linear/=alpha;}
        } else {
        let z=control_depth(p).x;float4 rgba;
        rgba=shutter_color(p,z);
        linear=rgba.rgb;alpha=rgba.a;
        associated=float4(linear*alpha,alpha);
        if (frame.temporal.y>1.) {
            var total=associated;
            if (frame.temporal.x>0.) {total+=accumulation[full_i];}
            accumulation[full_i]=total;
            associated=total/(frame.temporal.x+1.);
            linear=total.rgb/max(total.a,REG_POSITIVE_EPSILON);
            alpha=associated.a;
        }
        }
        if (!final_sample) {return;}
        store_color(associated,linear,alpha,gid.xy,i,count,flags);
    }
    if (!final_sample || (flags&controls)==0u) {return;}
    let z=control_depth(p).x;var id=0u;if (z<1.) {id=control_ids(p).x&REG_INDIRECT_ID_MASK;}
    let perspective=frame.params.x*frame.params.y/(frame.params.y-z*(frame.params.y-frame.params.x));
    let metric=select(id==0u,0.,select((frame.camera_program.z&2u)!=0u,z/(1.-z),select(frame.flags.x>0.5,lerp(frame.params.x,frame.params.y,z),perspective)));
    var normal=float3(0.);var velocity=float2(0.);
    if (id!=0u && (flags&(REG_OUTPUT_NORMALS|REG_OUTPUT_FLOAT_NORMALS))!=0u) {
        normal=normalize((mul(float4(control_normals(p).xyz,0.),frame.view)).xyz);
    }
    if ((frame.camera_program.z&2u)==0u && id!=0u && (flags&(REG_OUTPUT_MOTION|REG_OUTPUT_FLOAT_MOTION))!=0u) {velocity=control_motion(p).xy;}
    if ((flags&REG_OUTPUT_FLOAT_DEPTH)!=0u) {pixels[REG_FLOAT_DEPTH_PLANE*count+i]=asuint(select(id==0u,REG_EXR_BACKGROUND_DEPTH,metric));}
    if ((flags&REG_OUTPUT_FLOAT_NORMALS)!=0u) {
        for(var channel=0u;channel<3u;channel++) {pixels[(REG_FLOAT_NORMAL_PLANE+channel)*count+i]=asuint(normal[channel]);}
    }
    if ((flags&REG_OUTPUT_FLOAT_MOTION)!=0u) {
        let pixel_motion=velocity*float2(size);
        for(var channel=0u;channel<2u;channel++) {pixels[(REG_FLOAT_MOTION_PLANE+channel)*count+i]=asuint(pixel_motion[channel]);}
    }
    if ((flags&REG_OUTPUT_DEPTH_IDS)!=0u) {
        let quantized=select(id==0u,0u,uint(clamp(round(metric/frame.params.y*REG_UNORM16_MAX),1.,REG_UNORM16_MAX)));
        pixels[count+i]=(min(id,REG_UNORM16_MAX_U32)<<16u)|quantized;
    }
    if ((flags&REG_OUTPUT_NORMALS)!=0u) {
        let encoded=select(id!=0u,normal*.5+.5,float3(0.));
        pixels[count*2u+i]=pack_unorm(float4(encoded,select(id!=0u,1.,0.)));
    }
    if ((flags&REG_OUTPUT_MOTION)!=0u) {
        let encoded=uint2(round(clamp(velocity*.5+.5,float2(0.),float2(1.))*REG_UNORM16_MAX));
        pixels[count*3u+i]=encoded.x|(encoded.y<<16u);
    }
}

// Resolve filtering and all shutter samples before any nonlinear highlight extraction.
[shader("compute")] [numthreads(REG_COMPUTE_TILE,REG_COMPUTE_TILE,1)]
void post_source(uint3 gid : SV_DispatchThreadID) {
    let size=dimensions(color);
    if (any(gid.xy>=size)) {return;}
    let p=int2(gid.xy);let i=gid.y*size.x+gid.x;
    let z=control_depth(p).x;float4 rgba;
    rgba=shutter_color(p,z);
    var associated=float4(rgba.rgb*rgba.a,rgba.a);
    if (frame.temporal.x>0.) {associated+=accumulation[i];}
    if (!all(isfinite(associated))) {InterlockedMax(output_errors[0],REG_RADIANCE_NONFINITE);}
    accumulation[i]=associated;
}

[shader("compute")] [numthreads(REG_COMPUTE_TILE,REG_COMPUTE_TILE,1)]
void resolve_color(uint3 gid : SV_DispatchThreadID) {resolve_pixel(gid);}

[shader("compute")] [numthreads(REG_COMPUTE_TILE,REG_COMPUTE_TILE,1)]
void resolve_fused(uint3 gid : SV_DispatchThreadID) {resolve_pixel(gid);}

[shader("compute")] [numthreads(REG_COMPUTE_TILE,REG_COMPUTE_TILE,1)]
void resolve_fused_single(uint3 gid : SV_DispatchThreadID) {resolve_pixel(gid);}
