float3 eye(float t) { float u=saturate((t-12.)/12.); float v=u*u*(3.-2.*u); float pi=3.141592653589793; return float3(-2+6*v, 0+5*v, 8+16*v); }
struct Inputs {};
void compute(ComputeContext c, Inputs i) {
 uint block=c.id.x;uint count=0;
 float3 camera=eye(c.time+c.dt);
 [loop] for(uint id=block*256;id<min((block+1)*256,1000000u);id++) {
  bool close=distance(translation_load(id),camera)<.9;
  far_scale_store(id,close?float3(0):float3(1));count+=close?1:0;
 }
 lod_counts_store(block,count);
}
