// Packed diffusion receivers shared by shade, filter and numerical tests.
// Each visible triangle retains its own position and covered substrate energy.
import decoupled_slots;
// RGB9E5 packs the auxiliary ambient irradiance; primary light retains FP32.
uint pack_rgb9e5(float3 value) {
    let c=clamp(value,float3(0.),float3(65408.));
    let peak=max(c.r,max(c.g,c.b));
    if (peak<exp2(-24.)) {return 0u;}
    var e=uint(clamp(floor(log2(peak))+16.,0.,31.));
    var scale=exp2(float(e)-24.);
    if (round(peak/scale)>=512. && e<31u) {e++;scale*=2.;}
    let m=uint3(round(c/scale));
    return min(m.r,511u)|(min(m.g,511u)<<9u)|(min(m.b,511u)<<18u)|(e<<27u);
}
float3 unpack_rgb9e5(uint p) {
    return float3(float(p&511u),float((p>>9u)&511u),float((p>>18u)&511u))*exp2(float(p>>27u)-24.);
}

// A zero-albedo channel removes no energy but still donates its covered light.
float3 resolve_diffusion(float3 light_albedo, float3 albedo, float3 light, float coverage) {
    let lit=albedo>0.;let a=select(lit,albedo,float3(1.));
    return select(lit,light_albedo/a,light/max(coverage,1e-30));
}

struct DiffusionReceiver {float3 light;float3 ambient;float3 radius;float3 albedo;float3 position;float coverage;float geometric_coverage;};
DiffusionReceiver load_diffusion(StructuredBuffer<uint> records, uint r) {
    let coverage=asfloat(records[r+10u]);
    if (coverage<=0.) {return (DiffusionReceiver)0;}
    let rg=unpackHalf2x16ToFloat(records[r+3u]);let ba=unpackHalf2x16ToFloat(records[r+4u]);
    let gb=unpackHalf2x16ToFloat(records[r+5u]);
    return DiffusionReceiver(float3(asfloat(records[r]),asfloat(records[r+1u]),asfloat(records[r+2u])),unpack_rgb9e5(records[r+6u]),float3(rg,ba.x),float3(ba.y,gb),float3(asfloat(records[r+7u]),asfloat(records[r+8u]),asfloat(records[r+9u])),coverage,asfloat(records[r+11u]));
}
void clear_diffusion_record(RWStructuredBuffer<uint> records, uint r) {records[r+10u]=0u;}
void store_diffusion(RWStructuredBuffer<uint> records, uint r, DiffusionReceiver c) {
    records[r]=asuint(c.light.x);records[r+1u]=asuint(c.light.y);records[r+2u]=asuint(c.light.z);
    records[r+3u]=packHalf2x16(c.radius.xy);
    records[r+4u]=packHalf2x16(float2(c.radius.z,c.albedo.x));records[r+5u]=packHalf2x16(c.albedo.yz);
    records[r+6u]=pack_rgb9e5(c.ambient);records[r+10u]=asuint(c.coverage);records[r+11u]=asuint(c.geometric_coverage);
    records[r+7u]=asuint(c.position.x);records[r+8u]=asuint(c.position.y);records[r+9u]=asuint(c.position.z);
}
// Radial PDF (Christensen–Burley); the surface profile restores r/distance.
float3 radial_pdf(float r, float3 d) {
    let tail=exp(-r/(3.*d));return (tail+tail*tail*tail)/(4.*d);
}
struct DiffusionTap {float3 light;float3 ambient;float3 weight;};
DiffusionTap diffusion_self(DiffusionReceiver center, float r, float proposal) {
    let w=radial_pdf(r,max(center.radius,float3(1e-8)))/max(proposal,1e-20);
    return DiffusionTap(center.light*w,center.ambient*w,w);
}
DiffusionTap diffusion_sample(DiffusionReceiver center, DiffusionReceiver other, float r, float proposal, float footprint=0.) {
    let delta=other.position-center.position;
    // Integrate a finite donor footprint at the profile singularity. Each
    // covered position stays separate, including folds of the same object.
    let distance=max(sqrt(dot(delta,delta)+footprint*footprint),1e-8);
    let w=radial_pdf(distance,max(center.radius,float3(1e-8)))/max(proposal,1e-20)*(r/distance);
    return DiffusionTap(other.light*w,other.ambient*w,w);
}
DiffusionTap diffusion_tap(StructuredBuffer<uint> slots, StructuredBuffer<uint> records, SlotLayout layout, uint pixel, uint object, DiffusionReceiver center, float r, float proposal, float footprint=0.) {
    var albedo=float3(0.);var coverage=0.;var geometric_coverage=0.;
    var covered=(DiffusionTap)0;var substrate=(DiffusionTap)0;
    for (var at=layout.first(slots,pixel);at!=0xffffffffu;at=layout.next(slots,pixel,at)) {
        if (slots[at]!=object) {continue;}
        let c=load_diffusion(records,layout.diffusion(at));
        if (c.coverage<=0.) {continue;}
        let tap=diffusion_sample(center,c,r,proposal,footprint);
        albedo+=c.albedo;coverage+=c.coverage;geometric_coverage+=c.geometric_coverage;
        substrate.light+=tap.light*c.albedo;substrate.ambient+=tap.ambient*c.albedo;substrate.weight+=tap.weight*c.albedo;
        covered.light+=tap.light*c.coverage;covered.ambient+=tap.ambient*c.coverage;covered.weight+=tap.weight*c.coverage;
    }
    if (coverage<=0.) {return diffusion_self(center,r,proposal);}
    let local=diffusion_self(center,r,proposal);let mass=saturate(geometric_coverage);
    return DiffusionTap(lerp(local.light,resolve_diffusion(substrate.light,albedo,covered.light,coverage),mass),
        lerp(local.ambient,resolve_diffusion(substrate.ambient,albedo,covered.ambient,coverage),mass),
        lerp(local.weight,resolve_diffusion(substrate.weight,albedo,covered.weight,coverage),mass));
}

// Bilinear weights apply to complete taps, never to IDs or surface positions.
DiffusionTap diffusion_footprint(StructuredBuffer<uint> slots, StructuredBuffer<uint> records, SlotLayout layout, uint2 size, float2 q, uint object, DiffusionReceiver center, float r, float proposal, float footprint) {
    let base=int2(floor(q));let f=frac(q);var result=(DiffusionTap)0;
    for (uint y=0u;y<2u;y++) {for (uint x=0u;x<2u;x++) {
        let p=base+int2(x,y);let w=select(x==0u,1.-f.x,f.x)*select(y==0u,1.-f.y,f.y);
        var tap=diffusion_self(center,r,proposal);
        if (all(p>=int2(0)) && all(p<int2(size))) {
            tap=diffusion_tap(slots,records,layout,uint(p.y)*size.x+uint(p.x),object,center,r,proposal,footprint);
        }
        result.light+=w*tap.light;result.ambient+=w*tap.ambient;result.weight+=w*tap.weight;
    }}
    return result;
}
