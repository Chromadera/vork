// Sparse retained irradiance field: canonical bricks of L1 total irradiance
// (local transport plus distant visibility) and octahedral depth moments.
// The metadata buffer stores one header followed by a sorted brick directory.
import vork_constants;
import probe_visibility_policy;
import vork_prelude;
import vork_sampling;
import openpbr_core;
public struct ProbeGrid {
    // xyz: the first probe's position; w: intensity.
    public float4 origin;
    // xyz: probe spacing; w: capture distance.
    public float4 spacing;
    // xyz: resolution; w: rays per probe.
    public uint4 counts;
    // Normal and view offsets in metres, facing_bias, validity_threshold.
    public float4 bias;
    // Header x: exposure strata; y: optional reference-frame record; z: request dictionary offset.
    // Brick xyz: signed canonical key. Header w: mesh emitter topology.
    public uint4 visibility_policy;
    // Header: brick count, position offset, unique probe count, max dyadic level.
    // Brick: reserved, corner-index table offset, reserved, dyadic level.
    // Position: xyz in origin; dyadic level in field.w; canonical identity in counts.
    // Separate words: requests update x and y atomically, and Metal cannot take
    // an atomic on a vector element.
    public uint field_x;
    public uint field_y;
    public uint field_z;
    public uint field_w;
    public property uint4 field {get {return uint4(field_x,field_y,field_z,field_w);}}
};
// Stored total diffuse E/pi, including local transport and distant visibility.
// Layout matches TotalDiffuseIrradianceRecord in probes.rs. Capture writes its
// vec4 channels independently; this type owns their transport meaning.
public struct TotalDiffuseIrradianceRecord {
    public float4 channels[3];
    public float4 support;
};
// Group 2: hit kernels leave it free (bindings 0-2); colour stages append the
// volume to their shadow group, after its entries (VORK_PROBE_BINDING; wgpu
// numbers Vulkan bindings densely and passthrough SPIR-V is not remapped).
#ifndef VORK_PROBE_BINDING
#define VORK_PROBE_BINDING 0
#endif
#ifdef VORK_PROBE_IRRADIANCE_REGISTER
[[vk::binding(VORK_PROBE_BINDING,2)]] public StructuredBuffer<TotalDiffuseIrradianceRecord> probe_irradiance : register(VORK_PROBE_IRRADIANCE_REGISTER);
[[vk::binding(VORK_PROBE_BINDING+1,2)]] public StructuredBuffer<float2> probe_visibility : register(VORK_PROBE_VISIBILITY_REGISTER);
[[vk::binding(VORK_PROBE_BINDING+2,2)]] public RWStructuredBuffer<ProbeGrid> probe_grid_data : register(VORK_PROBE_GRID_REGISTER);
#else
[[vk::binding(VORK_PROBE_BINDING,2)]] public StructuredBuffer<TotalDiffuseIrradianceRecord> probe_irradiance;
[[vk::binding(VORK_PROBE_BINDING+1,2)]] public StructuredBuffer<float2> probe_visibility;
[[vk::binding(VORK_PROBE_BINDING+2,2)]] public RWStructuredBuffer<ProbeGrid> probe_grid_data;
#endif
public ProbeGrid probe_grid() {return probe_grid_data[0];}
// Readers without an installed volume bind a zeroed grid; an installed volume
// traces at least one ray per probe even when no brick is resident.
public bool probe_volume_installed() {return probe_grid().counts.w!=0u;}

public float3 probe_position_index(uint probe) {return probe_grid_data[probe_grid().field.y+probe].origin.xyz;}
public float4 probe_spacing(uint probe) {return probe_grid().spacing*exp2(float(probe_grid_data[probe_grid().field.y+probe].field.w));}
// One complete receiver stencil, selected from fine to coarse spatial levels.
// There is no fallback ray integral. A black supported sample remains distinguishable
// from an unsupported owned point and from a point outside the field.
public struct ProbeFieldSample {
    public float3 radiance;
    public float weight;
    public float support;
    public bool owned;
    public bool valid;
};
float3 probe_rotate(float3 v,float4 q) {return v+2.*cross(q.xyz,cross(q.xyz,v)+q.w*v);}
public float3 probe_world_vector(float3 v) {
    let at=probe_grid().visibility_policy.y;
    return at==0u ? v : probe_rotate(v,probe_grid_data[at].spacing);
}
float3 probe_local_vector(float3 v) {
    let at=probe_grid().visibility_policy.y;
    if(at==0u) {return v;}
    let q=probe_grid_data[at].spacing;return probe_rotate(v,float4(-q.xyz,q.w));
}
float3 probe_local_position(float3 p) {
    let at=probe_grid().visibility_policy.y;
    return at==0u ? p : probe_local_vector(p-probe_grid_data[at].origin.xyz);
}
int3 probe_brick_key(float3 p,uint level) {
    let g=probe_grid();let extent=g.spacing.xyz*exp2(float(level))*float3(max(int3(g.counts.xyz)-1,int3(1)));
    return int3(floor(probe_local_position(p)/extent));
}
bool probe_key_less(int3 a,int3 b) {return a.x<b.x || (a.x==b.x && (a.y<b.y || (a.y==b.y && a.z<b.z)));}
// Finest complete resident stencil wins. Sparse holes at a finer level are
// not owners: continue to coarser support rather than returning black.
uint probe_brick(float3 world,float3 normal,float3 view) {
    [loop] for(uint level=0u;level<=probe_grid().field.w;level++) {
        let p=world+(normal*probe_grid().bias.x+view*probe_grid().bias.y)*exp2(float(level));
        let key=probe_brick_key(p,level);uint lo=0u;uint hi=probe_grid().field.x;
        [loop] while(lo<hi) {
            let mid=lo+(hi-lo)/2u;let g=probe_grid_data[mid+1u];
            let other=asint(g.visibility_policy.xyz);
            if(g.field.w<level || (g.field.w==level && probe_key_less(other,key))) {lo=mid+1u;} else {hi=mid;}
        }
        if(lo>=probe_grid().field.x) {continue;}
        let g=probe_grid_data[lo+1u];
        if(g.field.w!=level || any(asint(g.visibility_policy.xyz)!=key)) {continue;}
        if(g.field.x!=0u || (probe_grid().field.w==0u && probe_grid().visibility_policy.z==0u)) {return lo;}
        let res=int3(g.counts.xyz);
        let base=clamp(int3(floor(probe_local_vector(p-g.origin.xyz)/g.spacing.xyz)),int3(0),max(res-2,int3(0)));
        var complete=true;
        for(uint i=0u;i<8u;i++) {
            let q=min(base+int3(int(i&1u),int((i>>1u)&1u),int((i>>2u)&1u)),res-1);
            let local=uint(q.x)+g.counts.x*(uint(q.y)+g.counts.y*uint(q.z));
            complete=complete && probe_grid_data[g.field.y+local/4u].field[local%4u]!=0xffffffffu;
        }
        if(complete) {return lo;}
    }
    return 0xffffffffu;
}
// Missing support is provisional. Requests share the retained metadata allocation;
// the exposure copies this dictionary before any replay can replace it.
void probe_request(float3 world,float3 normal,float3 view) {
    let at=probe_grid().visibility_policy.z;
    if(at==0u) {return;}
    uint ignored;
    InterlockedOr(probe_grid_data[at].field_x,1u,ignored);
    let level=probe_grid_data[at].field.w;
    let scale=exp2(float(level));
    let p=world+(normal*probe_grid().bias.x+view*probe_grid().bias.y)*scale;
    let cell=probe_brick_key(p,level);
    let capacity=probe_grid_data[at].field.z;
    let hash=pixel_hash(asuint(cell.x))^pixel_hash(asuint(cell.y)+1u)^pixel_hash(asuint(cell.z)+2u);
    // Bounded hash work is only scheduling: a collision requests a larger
    // dictionary and rejects the exposure; no receiver can be discarded.
    [loop] for(uint i=0u;i<min(capacity,REG_PROBE_REQUEST_PROBES);i++) {
        let slot=at+1u+(hash+i)%capacity;
        uint state;
        InterlockedCompareExchange(probe_grid_data[slot].field_x,0u,1u,state);
        if(state==0u) {
            probe_grid_data[slot].counts.xyz=asuint(cell);
            DeviceMemoryBarrier();
            InterlockedExchange(probe_grid_data[slot].field_x,2u,ignored);
            return;
        }
        DeviceMemoryBarrier();
        if(state==2u && all(probe_grid_data[slot].counts.xyz==asuint(cell))) {return;}
    }
    InterlockedOr(probe_grid_data[at].field_y,1u,ignored);
}
// E/pi toward `n` from one probe's record (a + b.n per channel).
float3 probe_record_lighting(uint probe,float3 direction,float l1) {
    let record=probe_irradiance[probe];let m=float4(1.,direction*l1);
    return max(float3(dot(record.channels[0],m),dot(record.channels[1],m),dot(record.channels[2],m)),float3(0.));
}
// Octahedral texel of the depth moments; texels beyond an edge fold onto the
// mirrored texel of that edge, as the octahedral map's border does.
float2 probe_visibility_texel(uint probe,int2 t) {
    let side=int(REG_PROBE_VISIBILITY_SIDE);var q=t;
    if (q.x<0) {q=int2(0,side-1-q.y);} else if (q.x>=side) {q=int2(side-1,side-1-q.y);}
    if (q.y<0) {q=int2(side-1-q.x,0);} else if (q.y>=side) {q=int2(side-1-q.x,side-1);}
    q=clamp(q,int2(0),int2(side-1));
    return probe_visibility[probe*REG_PROBE_VISIBILITY_SIDE*REG_PROBE_VISIBILITY_SIDE+uint(q.y*side+q.x)];
}
// Mean distance and mean squared distance along `direction` from the probe.
public float2 probe_moments(uint probe,float3 direction) {
    let side=float(REG_PROBE_VISIBILITY_SIDE);
    let c=(encode_octahedron(direction)*.5+.5)*side-.5;let base=int2(floor(c));let f=c-float2(base);
    let a=lerp(probe_visibility_texel(probe,base),probe_visibility_texel(probe,base+int2(1,0)),f.x);
    let b=lerp(probe_visibility_texel(probe,base+int2(0,1)),probe_visibility_texel(probe,base+int2(1,1)),f.x);
    return lerp(a,b,f.y);
}
// Select one stencil and return radiance, accepted weight and support status.
// Capture validity and the consumer's installed-TLAS connection policy decide
// acceptance together. Rejected corners enter neither energy nor normalization.
public ProbeFieldSample probe_field_sample<S:IProbeVisibility>(float3 world,float3 n,float3 view,float3 direction,float l1,float3 geometric,float3 position_error) {
    ProbeFieldSample result=ProbeFieldSample(float3(0.),0.,0.,false,false);
    if(probe_grid().field.x==0u) {return result;}
    let brick=probe_brick(world,n,view);if(brick==0xffffffffu) {probe_request(world,n,view);return result;}
    result.owned=true;
    let g=probe_grid_data[brick+1u];let res=int3(g.counts.xyz);
    let geometric_visibility=S.probe_rays;
    let p=world+n*g.bias.x+view*g.bias.y;
    let c=probe_local_vector(p-g.origin.xyz)/g.spacing.xyz;
    let base=clamp(int3(floor(c)),int3(0),max(res-2,int3(0)));
    let f=saturate(c-float3(base));
    var sum=float3(0.);var weights=0.;var valid_weights=0.;
    for (uint i=0u;i<8u;i++) {
        let o=int3(int(i&1u),int((i>>1u)&1u),int((i>>2u)&1u));
        let q=min(base+o,res-1);let local=uint(q.x)+g.counts.x*(uint(q.y)+g.counts.y*uint(q.z));
        let probe=probe_grid_data[g.field.y+local/4u].field[local%4u];
        if(probe==0xffffffffu) {continue;} // no resident vertex; never a visibility proof
        let corner=select(o==int3(1),f,float3(1.)-f);let trilinear=corner.x*corner.y*corner.z;
        if (trilinear==0. || probe_irradiance[probe].support.x<g.bias.w) {continue;}
        if (geometric_visibility && !S.probe_visible(world,geometric,position_error,probe_position_index(probe))) {continue;}
        let irradiance=probe_record_lighting(probe,direction,l1);
        valid_weights+=trilinear;
        var w=1.;
        if (!geometric_visibility) {
            let to_probe=probe_position_index(probe)-p;let distance=length(to_probe);
            let direction=to_probe/max(distance,REG_POSITIVE_EPSILON);
            // DDGI leak-control approximations: wrapped facing floor, cubed
            // Chebyshev bound and weak-weight crushing are heuristic, not exact
            // visibility or numerical error bounds.
            let wrap=(dot(direction,n)+1.)*.5;
            w=lerp(1.,wrap*wrap+.2,g.bias.z);
            let m=probe_moments(probe,-direction);
            if (distance>m.x) {
                let variance=abs(m.y-m.x*m.x);let d=distance-m.x;
                let chebyshev=variance/(variance+d*d);
                w*=max(chebyshev*chebyshev*chebyshev,0.);
            }
            // Keep weak weights from being renormalised into leaks.
            w=max(w,REG_POSITIVE_EPSILON);
            if (w<.2) {w*=w*w/.04;}
        }
        sum+=trilinear*w*irradiance;weights+=trilinear*w;
    }
    result.support=valid_weights;result.weight=weights;
    result.valid=valid_weights>0. && weights>REG_POSITIVE_EPSILON;
    if(result.valid) {result.radiance=sum/weights*g.origin.w;}
    return result;
}
// Directional providers have independent density: one selected capture per
// brick. Descriptors follow the diffuse depth plane, then glossy-event L1
// rough endpoints and each provider's 8/4/2/1 octahedral levels. A texel stores local RGB + World visibility and
// mean distance + mean squared distance. World detail is sampled at the query,
// never blurred into the local capture or reconstructed from diffuse L1.
public uint probe_radiance_level(uint level) {
    uint offset=0u;for(uint i=0u;i<level;i++) {let side=REG_PROBE_RADIANCE_SIDE>>i;offset+=side*side;}return offset;
}
public uint probe_provider_base() {return probe_grid().field.z*REG_PROBE_VISIBILITY_SIDE*REG_PROBE_VISIBILITY_SIDE;}
int2 probe_radiance_pixel(uint level,int2 pixel) {
    let side=int(REG_PROBE_RADIANCE_SIDE>>level);var q=pixel;
    if(q.x<0) {q=int2(0,side-1-q.y);} else if(q.x>=side) {q=int2(side-1,side-1-q.y);}
    if(q.y<0) {q=int2(side-1-q.x,0);} else if(q.y>=side) {q=int2(side-1-q.x,side-1);}
    return clamp(q,int2(0),int2(side-1));
}
public uint probe_radiance_address(uint provider,uint level,int2 pixel) {
    let side=REG_PROBE_RADIANCE_SIDE>>level;let q=probe_radiance_pixel(level,pixel);
    return probe_provider_base()+probe_grid().field.x*(1u+REG_PROBE_RADIANCE_SH_PAIRS)+REG_PROBE_RADIANCE_PAIRS*(provider*REG_PROBE_RADIANCE_TEXELS+probe_radiance_level(level)+uint(q.y)*side+uint(q.x));
}
float4 probe_radiance_texel(uint provider,uint level,int2 pixel) {
    let at=probe_radiance_address(provider,level,pixel);return float4(probe_visibility[at],probe_visibility[at+1u]);
}
public float4 probe_radiance_level_sample(uint provider,uint level,float3 direction) {
    let side=float(REG_PROBE_RADIANCE_SIDE>>level);let uv=(encode_octahedron(direction)*.5+.5)*side-.5;
    let base=int2(floor(uv));let f=uv-float2(base);
    return lerp(lerp(probe_radiance_texel(provider,level,base),probe_radiance_texel(provider,level,base+int2(1,0)),f.x),lerp(probe_radiance_texel(provider,level,base+int2(0,1)),probe_radiance_texel(provider,level,base+int2(1,1)),f.x),f.y);
}
public float2 probe_provider_moments(uint provider,float3 direction) {
    let side=float(REG_PROBE_RADIANCE_SIDE);let uv=(encode_octahedron(direction)*.5+.5)*side-.5;
    let base=int2(floor(uv));let f=uv-float2(base);
    return lerp(lerp(probe_visibility[probe_radiance_address(provider,0u,base)+2u],probe_visibility[probe_radiance_address(provider,0u,base+int2(1,0))+2u],f.x),lerp(probe_visibility[probe_radiance_address(provider,0u,base+int2(0,1))+2u],probe_visibility[probe_radiance_address(provider,0u,base+int2(1,1))+2u],f.x),f.y);
}
// This endpoint contains glossy-event radiance, independently of the diffuse
// field. Glossy-only sources must survive at roughness one as well as mirrors.
public float3 probe_provider_rough(uint provider,float3 direction) {
    let at=probe_provider_base()+probe_grid().field.x+provider*REG_PROBE_RADIANCE_SH_PAIRS;
    let m=float4(1.,direction);
    return max(float3(dot(float4(probe_visibility[at],probe_visibility[at+1u]),m),dot(float4(probe_visibility[at+2u],probe_visibility[at+3u]),m),dot(float4(probe_visibility[at+4u],probe_visibility[at+5u]),m)),float3(0.));
}
public float3 probe_captured_radiance(uint provider,float3 world,float3 direction,float roughness,float3 terminal) {
    let descriptor=probe_visibility[probe_provider_base()+provider];
    if(descriptor.y<.5 || descriptor.x<0.) {return float3(0.);}
    let position=probe_position_index(uint(descriptor.x));let offset=world-position;
    var lookup=direction;let distance=probe_provider_moments(provider,direction).x;
    // Captured misses are distant. Only finite local distances define parallax.
    if(distance>0. && distance<probe_grid_data[provider+1u].spacing.w) {
        let projected=dot(offset,direction);let discriminant=projected*projected+distance*distance-dot(offset,offset);
        if(discriminant>0.) {let travel=-projected+sqrt(discriminant);if(travel>0.) {lookup=normalize(offset+travel*direction);}}
    }
    var visibility=1.;let receiver_distance=length(offset);
    if(receiver_distance>0.) {
        let m=probe_provider_moments(provider,offset/receiver_distance);
        if(receiver_distance>m.x) {
            let variance=abs(m.y-m.x*m.x);let delta=receiver_distance-m.x;
            let probability=variance/(variance+delta*delta);visibility=probability*probability*probability;
            if(visibility<.2) {visibility*=visibility*visibility/.04;}
        }
    }
    let lod=min(saturate(roughness)*3.,2.);let level=min(uint(lod),1u);let f=lod-float(level);
    let capture=lerp(probe_radiance_level_sample(provider,level,lookup),probe_radiance_level_sample(provider,level+1u,lookup),f);
    var radiance=capture.rgb+capture.a*terminal;
    if(roughness>2./3.) {radiance=lerp(radiance,probe_provider_rough(provider,lookup),saturate(roughness*3.-2.));}
    return radiance*visibility*probe_grid().origin.w;
}
public struct DirectionalFieldSample {public float3 radiance;public bool owned;public bool valid;};
public DirectionalFieldSample probe_directional_field(float3 world,float3 normal,float3 view,float3 direction,float roughness,float3 terminal) {
    var result=DirectionalFieldSample(float3(0.),false,false);
    if(probe_grid().field.x==0u) {return result;}
    let brick=probe_brick(world,normal,view);
    if(brick==0xffffffffu) {probe_request(world,normal,view);return result;}result.owned=true;
    let descriptor=probe_visibility[probe_provider_base()+brick];result.valid=descriptor.y>.5;
    result.radiance=probe_captured_radiance(brick,world,direction,roughness,terminal);
    return result;
}
