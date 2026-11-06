import vork;
import std.hair_bsdf;
import mesh_exit;
import vork_constants;
Hair unpack_hair(float4 a,float4 b) {Hair h;h.absorption_coefficient=a.xyz;h.roughness=a.w;h.radial_roughness=b.x;h.coat=b.y;h.ior=b.z;h.offset=b.w;return h;}
public float3 strand_direct(float4 a,float4 b,float3 t,float3 n,float3 wo,float3 wi,out float pdf) {return hair_evaluate(unpack_hair(a,b),t,n,wo,wi,pdf);}
struct StrandGeometry : IExitGeometry {
    uint2 base;
    __init(uint2 base) {this.base=base;}
    bool bounded() {return true;}
    uint4 word(uint node,uint field) {
        let at=base.x+node*(REG_EXIT_NODE_VEC4S*4u)+field*4u;
        return uint4(vork_input_word(at),vork_input_word(at+1u),vork_input_word(at+2u),vork_input_word(at+3u));
    }
    float3 vertex(uint triangle,uint corner) {
        let at=base.y+triangle*(REG_EXIT_TRIANGLE_VEC4S*4u)+corner*4u;
        return float3(asfloat(vork_input_word(at)),asfloat(vork_input_word(at+1u)),asfloat(vork_input_word(at+2u)));
    }
};
public float3 strand_shadow(float3 p,float3 n,float3 direction,uint base,uint strand) {
    let d=normalize(direction);
    let header=base+strand*4u;
    let roots=uint2(base+vork_input_word(header),base+vork_input_word(header+2u));
    let count=vork_input_word(header+1u);
    let geometry=StrandGeometry(roots);
    let lo=asfloat(geometry.word(0u,0u).xyz);let hi=asfloat(geometry.word(0u,1u).xyz);
    let error=REG_FP32_GAMMA7*(abs(p)+abs(hi-lo));
    let epsilon=max(error.x,max(error.y,error.z));
    if (dot(n,d)>=0.) return d*epsilon;
    let hit=mesh_exit<StrandGeometry>(geometry,count,p,d,REG_FP32_MAX,1e-6);
    return d*((hit.found ? hit.distance : 0.)+epsilon);
}
public float3 strand_world(float4 a,float4 b,float3 t,float3 n,float3 wo,float3 world) {return world*hair_uniform_world(unpack_hair(a,b),t,n,wo);}

public float3 strand_sample(float4 a,float4 b,float3 t,float3 n,float3 wo,float3 u,out float3 direction,out bool reflected,out float pdf) {
    let sample=hair_sample(unpack_hair(a,b),t,n,wo,u);
    direction=sample.direction;reflected=sample.order==0u;pdf=sample.pdf;
    return sample.response/max(sample.pdf,1e-20);
}

public float3 strand_neighbours(float4 a,float4 b,float3 t,float3 wo,float3 wi,float behind) {
    if (behind<=0.) {return float3(0.);}
    t=normalize(t);let so=clamp(dot(wo,t),-1.,1.);let si=clamp(dot(wi,t),-1.,1.);
    let vo=(wo-t*so)/sqrt(max(1.-so*so,1e-8));let vi=(wi-t*si)/sqrt(max(1.-si*si,1e-8));
    let phi=atan2(dot(t,cross(vo,vi)),dot(vo,vi));
    if (cos(phi)<=0.) {return float3(0.);}
    let m=hair_transport_moments(unpack_hair(a,b),sin(.5*(asin(so)-asin(si))));
    return hair_bundle_backscatter(m,behind,si,so,phi,float3(0.),float3(0.),false);
}
// Hemisphere-integrated forward energy of the same Chiang material. The retained
// field is a low-frequency beam approximation, independent of receiver colour.
public float3 strand_forward(float4 a,float4 b,float3 tangent,float3 incident) {
    return hair_transport_moments(unpack_hair(a,b),dot(normalize(tangent),incident)).forward;
}

public float3 strand_forward_at(float4 a,float4 b,float3 tangent,float3 normal,float3 outgoing) {
    return hair_forward_at(unpack_hair(a,b),tangent,normal,outgoing);
}
