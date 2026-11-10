// Exceptional reconstruction runs in a separate kernel so its live state does
// not raise the register footprint of ordinary simulation and density passes.
#include "fluid/volume.slang"

// Distance-based reconstruction for a density family whose compact support
// cannot contain the ledger. Zhu--Bridson weighted centres retain isolated
// spheres and thin parcels. A two-cell exterior band bounds the correction;
// it cannot lower unrelated constant air or fill the whole domain.
float distance_field(float3 p) {
 if(container(p)>=0.) return asfloat(0x7f800000u);
 float dx=2.*DOMAIN/float(surface_dims.x);
 float parcel=MARKER_SPACING*pow(3./(4.*3.141592653589793),1./3.);
 float radius=max(KERNEL_SUPPORT,parcel+2.*dx);
 int3 lo,hi;query_bins(p,radius,lo,hi);
 float weight=0.;float3 centroid=0.;
 for(int z=lo.z;z<=hi.z;z++) for(int y=lo.y;y<=hi.y;y++) for(int x=lo.x;x<=hi.x;x++) {
  for(uint key=bins_load(bin_index(int3(x,y,z)));key!=0xffffffffu;key=links_load(key)) {
   if(landed_load(key)>0u) continue;
   float3 delta=markers_load(key)-p;
   float q=max(0.,1.-dot(delta,delta)/(radius*radius));
   float w=q*q*q;weight+=w;centroid+=w*delta;
  }
 }
 return weight>0. ? length(centroid/weight)-parcel : 2.*DOMAIN;
}
void compute(ComputeContext ctx,Inputs inputs) {
    uint id=ctx.id.x;int stage=inputs.stage;
    if(stage==37) {
        // Only a conservative lower bound failed. Evaluate the exact capacity
        // here, outside the ordinary volume/derivative kernel.
        if(correction_load(6)!=-1. || id>=volume_count) return;
        uint x=id%surface_dims.x,z=id/surface_dims.x;float sum=0.;
        float dx=2.*DOMAIN/float(surface_dims.x);
        if(x+1u<surface_dims.x && z+1u<surface_dims.z) {
            for(uint y=0;y+1u<surface_dims.y;y++) {
                float f[8];float minimum=1e30,maximum=-1e30;
                for(uint k=0;k<8;k++) {
                    int3 c=int3(x+(k&1u),y+((k>>1u)&1u),z+(k>>2u));
                    uint at=uint(c.x)+surface_dims.x*(uint(c.y)+surface_dims.y*uint(c.z));
                    float3 q=(float3(c)+.5)*dx+DOMAIN_ORIGIN-DOMAIN_EXTENT;
                    float source=surface_load(at),wall=container(q);
                    if(any(c==int3(0)) || any(c==int3(surface_dims)-1)) wall=max(wall,dx);
                    f[k]=isfinite(source) && source>=2.*DOMAIN ? max(source,wall) : wall;
                    minimum=min(minimum,f[k]);maximum=max(maximum,f[k]);
                }
                if(minimum>=0.) continue;
                sum+=maximum<=0. ? 1. : occupied_cell(f);
            }
        }
        volume_store(id,float3(sum,0.,0.));return;
    }
    if(stage==38) {
        if(id!=0 || correction_load(6)!=-1.) return;
        float capacity=0.;for(uint j=0;j<volume_total_count;j++) capacity+=volume_total_load(j).x;
        float film=0.;for(uint j=0;j<film_total_count;j++) film+=film_total_load(j);
        float film_dx=2.*DOMAIN/float(wetness_dims.x),dx=2.*DOMAIN/float(surface_dims.x);
        uint particles=0u;for(uint j=0;j<mass_blocks_count;j++) particles+=mass_blocks_load(j);
        float target=float(particles)*pow(MARKER_SPACING/dx,3.);
        if(capacity+film*film_dx*film_dx/(dx*dx*dx)<target-max(target*0.0001,0.000001)) correction_store(3,-1.);
        correction_store(6,0.);return;
    }
    if(stage==34) {
        // One item per column keeps an unused fallback independent of the
        // display height. Only failed density capacity enters the gather.
        if(correction_load(3)>=0. || id>=volume_count) return;
        uint x=id%surface_dims.x,z=id/surface_dims.x;
        float dx=2.*DOMAIN/float(surface_dims.x);
        for(uint y=0;y<surface_dims.y;y++) {
            float3 q=(float3(x,y,z)+.5)*dx+DOMAIN_ORIGIN-DOMAIN_EXTENT;
            surface_store(x+surface_dims.x*(y+surface_dims.y*z),distance_field(q));
        }
        return;
    }
    if(stage==35) {
        if(id!=0 || correction_load(3)>=0.) return;
        float dx=2.*DOMAIN/float(surface_dims.x);
        correction_store(0,-dx);correction_store(1,-dx);
        correction_store(2,MARKER_SPACING+dx);correction_store(3,0.);
        correction_store(4,0.);correction_store(5,0.);correction_store(6,1.);
        return;
    }
}
