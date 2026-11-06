// Yu–Turk surface fitting from one retained particle generation.
// Separate from the solver so its covariance work does not inflate every solver dispatch.
import std.particles;

void compute(ComputeContext ctx,Inputs i) {uint id=ctx.id.x;

  if(id>=markers_count) return;
  float3 p=markers_load(id);if(landed_load(id)>0u) return;
  float radius=KERNEL_SUPPORT,neighbourhood=2.5*radius;
  int3 lo,hi;query_bins(p,neighbourhood,lo,hi);
  ParticleMoments moments={};
  for(int z=lo.z;z<=hi.z;z++) for(int y=lo.y;y<=hi.y;y++) for(int x=lo.x;x<=hi.x;x++) {
   for(uint key=bins_load(bin_index(int3(x,y,z)));key!=0xffffffffu;key=links_load(key)) {
    if(landed_load(key)>0u) continue;
    float3 relative=markers_load(key)-p;
    particle_moment(moments,relative,neighbourhood);
   }
  }
  float support;float3x3 metric=particle_metric(moments,radius,neighbourhood,25u,2.,support);
  // Fit shape without transporting material centres. Ensure a compressed
  // kernel still contains its marker's assigned rest-volume sphere.
  float parcel=MARKER_SPACING*pow(3./(4.*3.141592653589793),1./3.);
  float norm=sqrt(dot(metric[0],metric[0])+dot(metric[1],metric[1])+dot(metric[2],metric[2]));
  float widen=max(1.,parcel*norm);metric/=widen;support*=widen;
  centres_store(id,float4(p,support*1.00001+1e-7));metric_x_store(id,metric[0]);metric_y_store(id,metric[1]);metric_z_store(id,metric[2]);
  amplitude_store(id,(8./3.141592653589793)*abs(determinant(metric))*(MARKER_SPACING*MARKER_SPACING*MARKER_SPACING));
  bounds_atomic_min(6u,~asuint(support));
  uint3 bin=uint3(clamp(int3(floor((p-DOMAIN_ORIGIN+DOMAIN_EXTENT)*(.5*float(BIN_RES)/DOMAIN))),int3(0),int3(BIN_DIMS)-1));
  for(uint a=0;a<3;a++) {bounds_atomic_min(a,bin[a]);bounds_atomic_min(a+3u,~bin[a]);}
return;

}