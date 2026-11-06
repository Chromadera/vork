// Ihmsen et al., CGI 2012, equations 2–8; one-way diffuse material.
// Include after config and authored boundary callbacks. No floating atomics.
// SI-normalized thresholds and finite residence differ from the paper's presets.
uint white_hash(uint x) {x^=x>>16;x*=0x7feb352du;x^=x>>15;x*=0x846ca68bu;return x^(x>>16);}
float white_random(inout uint state) {state=white_hash(state);return float(state>>8)*(1./16777216.);}
float3 white_normal_at(float3 p) {
 float h=2.*MARKER_SPACING;int3 lo,hi;query_bins(p,h,lo,hi);float3 n=0.;
 for(int z=lo.z;z<=hi.z;z++) for(int y=lo.y;y<=hi.y;y++) for(int x=lo.x;x<=hi.x;x++)
 for(uint key=bins_load(bin_index(int3(x,y,z)));key!=0xffffffffu;key=links_load(key)) {
  if(landed_load(key)>0u) continue;
  float3 d=p-markers_load(key);float r=length(d);
  if(r>1e-8 && r<h) n+=d/r*(1.-r/h);
 }
 return length(n)>1e-5 ? normalize(n) : float3(0.);
}
void white_neighbours(float3 p,out uint count,out float3 flow) {
 float h=2.*MARKER_SPACING,weight=0.;int3 lo,hi;query_bins(p,h,lo,hi);count=0u;flow=0.;
 for(int z=lo.z;z<=hi.z;z++) for(int y=lo.y;y<=hi.y;y++) for(int x=lo.x;x<=hi.x;x++)
 for(uint key=bins_load(bin_index(int3(x,y,z)));key!=0xffffffffu;key=links_load(key)) {
  if(landed_load(key)>0u) continue;
  float r=length(p-markers_load(key));if(r>=h) continue;
  float w=1.-r/h;weight+=w;flow+=w*marker_velocity_load(key);count++;
 }
 flow/=max(weight,1e-8);
}
float white_potential(uint id) {
 float3 p=markers_load(id),v=marker_velocity_load(id),n=white_normal_load(id);
 float h=2.*MARKER_SPACING,air=0.,crest=0.;
 float speed=sqrt(max(GRAVITY*h+CAPILLARY/h,1e-12));
 float energy=.5*dot(v,v)/(speed*speed);if(energy<=.5) return 0.;
 int3 lo,hi;query_bins(p,h,lo,hi);
 for(int z=lo.z;z<=hi.z;z++) for(int y=lo.y;y<=hi.y;y++) for(int x=lo.x;x<=hi.x;x++)
 for(uint key=bins_load(bin_index(int3(x,y,z)));key!=0xffffffffu;key=links_load(key)) {
  if(key==id || landed_load(key)>0u) continue;
  float3 d=p-markers_load(key),dv=v-marker_velocity_load(key);float r=length(d),s=length(dv);
  if(r<=1e-8 || r>=h) continue;
  float w=1.-r/h;
  air+=s*(1.-dot(dv/max(s,1e-8),d/r))*w/speed;
  if(dot(d,n)>0.) crest+=(1.-dot(n,white_normal_load(key)))*w;
 }
 if(dot(v/max(length(v),1e-8),n)<.6) crest=0.;
 return saturate((energy-.5)/4.5)*(saturate((air-5.)/15.)+saturate((crest-2.)/6.));
}
float white_tau() {return sqrt(MARKER_SPACING/max(GRAVITY,1e-8));}
void white_advance(uint id,float3 p,float3 v,float life,float dt) {
 float tau=white_tau();uint neighbours;float3 flow;white_neighbours(p,neighbours,flow);
 uint kind=neighbours<6u ? 1u : (neighbours>20u ? 3u : 2u);
 if(kind==1u) v.y-=GRAVITY*dt;
 else if(kind==2u) v=flow;
 else {v=lerp(v,flow,1.-exp(-dt/tau));v.y+=GRAVITY*dt;}
 p+=dt*v;life-=dt;
 if(container(p)>=0. || any(abs(p-DOMAIN_ORIGIN)>=DOMAIN_EXTENT-MARKER_SPACING) || !all(isfinite(p))) life=0.;
 white_position_store(id,life>0. ? p : DOMAIN_ORIGIN);white_velocity_store(id,v);
 white_life_store(id,max(0.,life));white_kind_store(id,life>0. ? kind : 0u);
}
uint white_birth_proposal(uint rank) {
 uint lo=0u,hi=white_source_blocks_count;
 while(lo<hi) {uint mid=lo+(hi-lo)/2u;if(white_source_blocks_load(mid)>rank) hi=mid;else lo=mid+1u;}
 uint block=lo,base=block==0u ? 0u : white_source_blocks_load(block-1u);
 lo=block*256u;hi=min(lo+256u,white_birth_source_count);
 while(lo<hi) {uint mid=lo+(hi-lo)/2u;if(white_source_prefix_load(mid)>rank-base) hi=mid;else lo=mid+1u;}
 return lo;
}
void compute(ComputeContext ctx,Inputs inputs) {
 uint id=ctx.id.x;int stage=inputs.stage;
 if(stage==1) {
  if(id<markers_count) white_normal_store(id,landed_load(id)==0u ? white_normal_at(markers_load(id)) : float3(0.));
  return;
 }
 // A fixed physical proposal budget is independent of available secondary slots.
 // Potential is in [0,2]. Thinning uniform marker proposals therefore has the
 // same aggregate source rate, including when only a few markers are energetic.
 if(stage==5) {
  if(id>=white_birth_source_count) return;
  uint rng=white_hash(id^white_hash(ctx.frame)^white_hash(ctx.seed));
  uint source=min(uint(white_random(rng)*float(markers_count)),markers_count-1u);
  float potential=landed_load(source)==0u ? white_potential(source) : 0.;
  float proposals=ctx.dt*float(markers_count)/white_tau();
  bool born=white_random(rng)<.5*potential*saturate(proposals-float(id));
  white_birth_source_store(id,born ? source : 0xffffffffu);
  white_birth_potential_store(id,potential);return;
 }
 if(stage==6) {
  if(id>=white_source_blocks_count) return;
  uint count=0u;for(uint j=id*256u;j<min((id+1u)*256u,white_birth_source_count);j++) {
   count+=white_birth_source_load(j)!=0xffffffffu ? 1u : 0u;white_source_prefix_store(j,count);
  }
  white_source_blocks_store(id,count);return;
 }
 if(stage==7) {
  if(id!=0u) return;uint prefix=0u;
  white_emission_store(3u,0xffffffffu);
  for(uint b=0u;b<white_source_blocks_count;b++) {prefix+=white_source_blocks_load(b);white_source_blocks_store(b,prefix);}
  return;
 }
 if(stage==3 || stage==10) {
  if(id>=white_blocks_count) return;
  uint count=0u;for(uint j=id*256u;j<min((id+1u)*256u,white_position_count);j++) {
   bool member=stage==10 ? white_kind_load(j)>0u : white_kind_load(j)==0u;
   if(stage==3 && member && white_generation_load(j)>=(0xfffffffeu-j)/white_position_count) white_emission_atomic_min(3u,j);
   white_slot_rank_store(j,count);count+=member ? 1u : 0u;
  }
  white_blocks_store(id,count);return;
 }
 if(stage==8 || stage==11) {
  if(id!=0u) return;uint prefix=0u;
  for(uint b=0u;b<white_blocks_count;b++) {uint count=white_blocks_load(b);white_blocks_store(b,prefix);prefix+=count;}
  // An exhausted identity rejects point publication through the live-count contract.
  if(stage==11) white_count_store(0u,white_emission_load(3u)==0xffffffffu ? prefix : 0xffffffffu);
  else {
   uint requested=white_source_blocks_load(white_source_blocks_count-1u);
   uint allocated=white_emission_load(3u)==0xffffffffu ? min(prefix,requested) : 0u;
   white_emission_store(0u,requested);white_emission_store(1u,allocated);white_emission_store(2u,requested-allocated);
  }
  return;
 }
 if(id>=white_position_count) return;
 if(stage==0) {
  white_position_store(id,DOMAIN_ORIGIN);white_velocity_store(id,float3(0.));
  white_life_store(id,0.);white_kind_store(id,0u);white_generation_store(id,0u);return;
 }
 if(stage==2) {
  if(white_life_load(id)>0.) white_advance(id,white_position_load(id),white_velocity_load(id),white_life_load(id),ctx.dt);
  return;
 }
 if(stage==4) {
  if(white_kind_load(id)>0u || white_generation_load(id)>=(0xfffffffeu-id)/white_position_count) return;
  uint rank=white_blocks_load(id/256u)+white_slot_rank_load(id);
  if(rank>=white_emission_load(1u)) return;
  uint proposal=white_birth_proposal(rank),source=white_birth_source_load(proposal);
  float potential=white_birth_potential_load(proposal),tau=white_tau();
  uint rng=white_hash(proposal^white_hash(ctx.frame)^white_hash(ctx.seed)^0x706f7365u);
  float3 p=markers_load(source),v=marker_velocity_load(source);
  float3 axis=length(v)>1e-8 ? normalize(v) : float3(0,1,0);
  float3 e=normalize(cross(axis,abs(axis.y)<.9 ? float3(0,1,0) : float3(1,0,0))),f=cross(axis,e);
  float radius=.5*MARKER_SPACING*sqrt(white_random(rng)),angle=6.28318530718*white_random(rng);
  float3 radial=radius*(cos(angle)*e+sin(angle)*f);
  p+=radial+ctx.dt*white_random(rng)*v;v+=radial/tau;
  white_generation_store(id,white_generation_load(id)+1u);
  white_advance(id,p,v,tau*lerp(20.,80.,saturate(potential)),ctx.dt);return;
 }
 if(stage==12 && white_kind_load(id)>0u) {
  uint prefix=white_blocks_load(id/256u)+white_slot_rank_load(id);
  white_draw_position_store(prefix,white_position_load(id));
  white_draw_id_store(prefix,white_generation_load(id)*white_position_count+id);
 }
}
