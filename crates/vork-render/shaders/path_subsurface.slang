import orthonormal_basis;
// Same-object spatial BSSRDF sampling. Homogeneous coefficients are frozen at
// entry, following OpenPBR's MFP/albedo contract; no world-medium state is used.
// The boundary is index matched and redistributes exit directions diffusely.
// Albedo inversion: OpenPBR 1.1.1, subsurface, van de Hulst approximation.
import vork_constants;
import vork_prelude;
import vork_sampling;
import closure;
import path_sampler;
float subsurface_single_albedo(float color,float g) {
    if (color<=0.) {return 0.;}
    if (color>=1.) {return 1.;}
    // Solve .139*s^2-(1.139+1.17*C)*s+1-C=0 using its
    // cancellation-free small root rather than rounded inverse-fit constants.
    let b=1.139+1.17*color;
    let s=2.*(1.-color)/(b+sqrt(b*b-4.*.139*(1.-color)));
    let square=s*s;
    return clamp((1.-square)/(1.-g*square),0.,1.);
}
float3 subsurface_phase(float3 axis,float g,float2 u) {
    // Algebraically cancel g from the HG inverse. This retains the exact
    // isotropic limit without an arbitrary near-zero anisotropy threshold.
    let h=2.*u.x-1.;let square=g*g;let denominator=1.+g*h;
    let cosine=clamp((h*(1.+square)+.5*g*(3.-square+h*h*(1.+square)))
        /(denominator*denominator),-1.,1.);
    let sine=sqrt(max(1.-cosine*cosine,0.));let phi=REG_TAU*u.y;
    return basis_direction(axis,float3(sine*cos(phi),sine*sin(phi),cosine));
}
struct SubsurfaceBoundary {
    float t;uint object;uint instance;uint primitive;float2 barycentrics;float3x4 world_to_object;
};
SubsurfaceBoundary subsurface_boundary(RaytracingAccelerationStructure scene,StructuredBuffer<uint> words,uint offset,uint identity,float3 p,float3 d,float limit) {
    RayDesc ray;ray.Origin=p;ray.Direction=d;ray.TMin=0.;ray.TMax=limit;
#ifdef VORK_PROPAGATION
    // Adjacent segments own their shared endpoint. Native triangle queries
    // may exclude TMax and reject a zero-distance hit in the following segment.
    ray.TMax=limit<REG_FP32_MAX ? asfloat(asuint(limit)+1u) : limit;
#endif
    RayQuery<RAY_FLAG_FORCE_NON_OPAQUE> query;
    query.TraceRayInline(scene,RAY_FLAG_NONE,0xffu,ray);
    while (query.Proceed()) {
        if (query.CandidateType()==CANDIDATE_NON_OPAQUE_TRIANGLE
            && words[offset+query.CandidateInstanceIndex()]==identity && !query.CandidateTriangleFrontFace()
            && query.CandidateTriangleRayT()>0.) {query.CommitNonOpaqueTriangleHit();}
    }
    SubsurfaceBoundary result={};result.t=-1.;
    if (query.CommittedStatus()==COMMITTED_TRIANGLE_HIT) {
        result.t=query.CommittedRayT();result.object=query.CommittedInstanceID();result.instance=query.CommittedInstanceIndex();
        result.primitive=query.CommittedPrimitiveIndex();result.barycentrics=query.CommittedTriangleBarycentrics();
        result.world_to_object=query.CommittedWorldToObject3x4();
    }
    return result;
}
struct SubsurfaceWalk {SubsurfaceBoundary boundary;float3 direction;float3 weight;float length;bool local;};
interface ISubsurfaceSegments {
    static bool segment(float3 p,inout float3 d,inout bool escaped,inout float previous_distance,out float limit,out float3 transfer);
    static bool work(uint count);
};
SubsurfaceWalk subsurface_walk<S:ISubsurfaceSegments>(RaytracingAccelerationStructure scene,StructuredBuffer<uint> words,uint offset,uint identity,float3 p,float3 normal,Closure c,inout uint rng) {
    SubsurfaceWalk result={};result.boundary.t=-1.;
    let color=clamp(c.subsurface_color,float3(0.),float3(1.));
    let radius=c.subsurface_radius*c.subsurface_radius_scale;
    let active=select(radius>float3(0.),float3(1.),float3(0.));
    let count=active.x+active.y+active.z;
    // Exact zero-MFP limits form a local spectral component. Sample that
    // component separately; never replace tiny authored lengths by a floor.
    let local_probability=(3.-count)/3.;
    if (path_random(rng)<local_probability) {
        result.local=true;result.weight=(float3(1.)-active)*color/local_probability;return result;
    }
    let safe_radius=select(radius>float3(0.),radius,float3(1.));
    var weight=active/(1.-local_probability);
    let log_radius=log(safe_radius);
    let g=clamp(c.subsurface_scatter_anisotropy,-1.+REG_FP32_EPSILON,1.-REG_FP32_EPSILON);
    let albedo=float3(subsurface_single_albedo(color.x,g),subsurface_single_albedo(color.y,g),subsurface_single_albedo(color.z,g));
    let u=path_random(rng);let phi=REG_TAU*path_random(rng);
    var direction=basis_direction(-normal,float3(sqrt(u)*cos(phi),sqrt(u)*sin(phi),sqrt(1.-u)));
    var escaped=false;var previous_distance=0.;var work=0u;
    for (;;) {
        float limit;float3 transfer;
        if(!S.work(work++) || !S.segment(p,direction,escaped,previous_distance,limit,transfer)) return result;
        weight*=transfer;
        let boundary=subsurface_boundary(scene,words,offset,identity,p,direction,limit);
        if (boundary.t<0. && limit==REG_FP32_MAX) {return result;} // open meshes have no interior walk
        // A mixture of exponential free-flight laws has support for every
        // channel with current throughput. All RGB components follow the same geometric path, with
        // their exact spectral likelihood ratio instead of a hero-channel mask.
        let probabilities=weight/(weight.x+weight.y+weight.z);
        let choice=path_random(rng);
        let channel=choice<probabilities.x ? 0u : (choice<probabilities.x+probabilities.y ? 1u : 2u);
        let sampled=-log(path_random(rng))*safe_radius[channel];
        let exiting=boundary.t>=0. && boundary.t<=sampled;let distance=exiting ? boundary.t : min(sampled,limit);
        let scattering=!exiting && sampled<limit;
        result.length+=distance;
        var log_density=-distance/safe_radius;
        if (scattering) {log_density-=log_radius;}
        // Log-sum-exp normalization preserves arbitrary positive authored MFPs
        // without overflow in 1/r or underflow in the sampled flight's PDF.
        let positive=weight>float3(0.);
        let log_joint=select(positive,log_density+log(select(positive,weight,float3(1.))),float3(-REG_FP32_MAX));
        let maximum=max(log_joint.x,max(log_joint.y,log_joint.z));
        let density=exp(log_joint-float3(maximum))*select(positive,float3(1.),float3(0.));
        weight=density*((weight.x+weight.y+weight.z)/(density.x+density.y+density.z));
        if (exiting) {
            result.boundary=boundary;result.direction=direction;result.weight=weight;return result;
        }
        p+=direction*distance;
        if(!scattering) continue;
        escaped=false;previous_distance=0.;work=0u;weight*=albedo;
        // Compensated roulette, with no fixed collision/optical-depth cutoff.
        let survive=min(max(weight.x,max(weight.y,weight.z)),REG_SUBSURFACE_WALK_SURVIVAL);
        if (path_random(rng)>=survive) {return result;}
        weight/=survive;
        let a=path_random(rng);let b=path_random(rng);
        direction=subsurface_phase(direction,g,float2(a,b));
    }
}
