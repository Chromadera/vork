// Shared retained-receiver diffusion integral; no entry points or resource bindings.
import vork_constants;
import vork_sampling;
import subsurface_capture;
import decoupled_slots;
void diffuse_range(StructuredBuffer<uint> slots, StructuredBuffer<uint> records, SlotLayout layout, uint at, int2 p, uint2 size, float2 projection, bool orthographic, uint budget, uint phase, uint begin, uint end, inout float3 sum, inout float3 ambient_sum, inout float3 weights, out float3 color, out float3 ambient) {
    let c=load_diffusion(records,layout.diffusion(at));let object=slots[at];let d=c.radius;
    let max_d=max(d.r,max(d.g,d.b));color=float3(0.);ambient=float3(0.);
    if (max_d<=0.) {return;}
    let pixels_per_meter=float2(size)*abs(projection)*.5/select(orthographic,1.,-c.position.z);
    // One complete integral at the retained receiver. More exposure samples
    // increase density even when decoupled shading runs only once.
    let count=REG_SUBSURFACE_BASE_PROBES+budget;
    var sequence=sample_2d_sequence(uint2(p)+uint2(0u,phase*size.y),begin,count,REG_SAMPLING_STREAM_SUBSURFACE);
    let footprint=.5/max(pixels_per_meter.x,pixels_per_meter.y);
    for (uint i=begin;i<end;i++) {
        let u=sample_2d_next(sequence);
        // Invert F(r)=1-(exp(-r/d)+3*exp(-r/(3d)))/4.
        // With x=exp(-r/(3d)), x^3+3x=4(1-u). The real root is
        // 2*sinh(asinh(2*(1-u))/3); every radial stratum now covers
        // adjacent profile mass, rather than jumping between mixture lobes.
        let t=2.*max(1.-u.x,1e-7);
        let h=log(t+sqrt(t*t+1.))/3.;
        // Equivalent to 2*sinh(h), without tail cancellation.
        let x=2.*t/(exp(2.*h)+1.+exp(-2.*h));
        let r=-3.*max_d*log(min(x,1.));
        let angle=u.y*6.283185307179586;
        let q=float2(p)+float2(cos(angle),sin(angle))*r*pixels_per_meter;
        let proposal=radial_pdf(r,float3(max_d)).r;
        let tap=diffusion_footprint(slots,records,layout,size,q,object,c,r,proposal,footprint);
        sum+=tap.light;ambient_sum+=tap.ambient;weights+=tap.weight;
    }
    if (end<count) {return;}
    // Diffusion vanishes continuously below the represented pixel footprint.
    let resolved=smoothstep(float3(0.),float3(1.),d*max(pixels_per_meter.x,pixels_per_meter.y));
    color=(sum/max(weights,float3(1e-20))-c.light)*c.albedo*resolved;
    ambient=(ambient_sum/max(weights,float3(1e-20))-c.ambient)*c.albedo*resolved;
}

// Numerical callers can still evaluate the same complete integral in one call.
void diffuse(StructuredBuffer<uint> slots, StructuredBuffer<uint> records, SlotLayout layout, uint at, int2 p, uint2 size, float2 projection, bool orthographic, uint budget, uint phase, out float3 color, out float3 ambient) {
    var sum=float3(0.);var ambient_sum=float3(0.);var weights=float3(0.);
    diffuse_range(slots,records,layout,at,p,size,projection,orthographic,budget,phase,0u,REG_SUBSURFACE_BASE_PROBES+budget,sum,ambient_sum,weights,color,ambient);
}
