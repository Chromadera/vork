// Resource-free secondary-direction proposal, shared by production and numeric checks.
import vork_constants;
import vork_ray_sampling;
public struct GiProposal {
    public float3 axis[10];
    public float weight[10];
    public float total;
};
// A generated sample belongs to its selected component by construction. Carry
// that identity so floating-point cap-boundary tests cannot omit its density.
public float gi_proposal_pdf(GiProposal guide,float3 normal,float3 direction,uint sampled_cap=0xffffffffu) {
    let cosine=max(dot(normal,direction),0.)/REG_PI;
    if(guide.total<=0.) return cosine;
    var density=0.;
    for(uint i=0u;i<10u;i++) {
        if(i==sampled_cap || dot(guide.axis[i],direction)>=REG_GI_GUIDE_COSINE)
            density+=guide.weight[i]/guide.total/(2.*REG_PI*(1.-REG_GI_GUIDE_COSINE));
    }
    return (1.-REG_GI_GUIDE_MIX)*cosine+REG_GI_GUIDE_MIX*density;
}
public float3 gi_proposal_direction(GiProposal guide,float3 normal,float2 u,float2 strategy,out uint sampled_cap) {
    sampled_cap=0xffffffffu;
    if(guide.total<=0. || strategy.x>=REG_GI_GUIDE_MIX) {
        let disk=ray_concentric_disk(u);
        return mul(float3(disk,sqrt(max(1.-dot(disk,disk),0.))),ray_basis(normal));
    }
    var target=strategy.y*guide.total;var selected=0u;
    for(uint i=0u;i<10u;i++) {
        if(guide.weight[i]<=0.) continue;
        selected=i;target-=guide.weight[i];if(target<0.) break;
    }
    sampled_cap=selected;
    let z=1.-u.x*(1.-REG_GI_GUIDE_COSINE);let radius=sqrt(max(0.,1.-z*z));
    let phi=2.*REG_PI*u.y;
    return mul(float3(radius*cos(phi),radius*sin(phi),z),ray_basis(guide.axis[selected]));
}

