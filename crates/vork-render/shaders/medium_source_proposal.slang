// Resource-free direction mixture for finite medium source domains.
import vork_constants;
import vork_sampling;
import vork_ray_sampling;
internal extern uint medium_source_domains();
internal extern bool medium_source_domain(uint index);
internal extern float medium_source_cone(uint index,float3 from,out float3 axis);
// Half cosine, half a uniform mixture of every emitting domain's cone.
// Both continuation and explicit mesh-source MIS evaluate this full density.
public float medium_source_pdf(float3 from,float3 normal,float3 direction) {
    let cosine_pdf=max(dot(normal,direction),0.)/REG_PI;
    let domains=medium_source_domains();var sources=0u;var source_pdf=0.;
    for(uint i=0u;i<domains;i++) {
        if(!medium_source_domain(i)) {continue;}sources++;
        float3 axis;let width=medium_source_cone(i,from,axis);
        // Squared chord length avoids cancellation for small distant cones.
        let delta=axis-direction;
        if(width>0. && dot(delta,delta)<=2.*width) {source_pdf+=1./(2.*REG_PI*width);}
    }
    return sources==0u ? cosine_pdf : .5*(cosine_pdf+source_pdf/float(sources));
}
// Sample the cosine/domain mixture.
// Evaluate every proposal at the chosen direction, including overlapping cones.
public float medium_source_direction(float3 from,float3 normal,uint2 pixel,uint index,uint count,inout float3 direction) {
    let domains=medium_source_domains();var sources=0u;
    for(uint i=0u;i<domains;i++) {if(medium_source_domain(i)) {sources++;}}
    if(sources==0u) {return 1.;}
    let u=sample_2d(pixel,index,count,REG_SAMPLING_STREAM_MEDIUM_SOURCE);
    if(u.x<.5) {
        let choice=min(uint(u.x*2.*float(sources)),sources-1u);var next=0u;
        for(uint i=0u;i<domains;i++) {
            let at=i;if(!medium_source_domain(at)) {continue;}
            if(next++!=choice) {continue;}
            float3 axis;let width=medium_source_cone(at,from,axis);
            let height=(u.x*2.*float(sources)-float(choice))*width;let cosine=1.-height;
            let sine=sqrt(max(height*(2.-height),0.));let phi=2.*REG_PI*u.y;
            direction=mul(float3(sine*cos(phi),sine*sin(phi),cosine),ray_basis(axis));break;
        }
    }
    let pdf=medium_source_pdf(from,normal,direction);
    return pdf>0. ? max(dot(normal,direction),0.)/(REG_PI*pdf) : 0.;
}
