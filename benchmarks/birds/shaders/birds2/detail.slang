float3 eye(float t) {
 if(t<=7.) return lerp(float3(3,2.5,18),float3(2,1.8,12),saturate((t-6.)/1.));
 if(t<=8.) return lerp(float3(2,1.8,12),float3(1,0.8,6),saturate((t-7.)/1.));
 if(t<=9.) return lerp(float3(1,0.8,6),float3(0,0.3,1),saturate((t-8.)/1.));
 if(t<=10.) return lerp(float3(0,0.3,1),float3(-3,0.4,-1),saturate((t-9.)/1.));
 if(t<=11.) return lerp(float3(-3,0.4,-1),float3(-8,2,1),saturate((t-10.)/1.));
 if(t<=12.) return lerp(float3(-8,2,1),float3(-15,4,5),saturate((t-11.)/1.));
 if(t<=14.) return lerp(float3(-15,4,5),float3(-24,7,13),saturate((t-12.)/2.));
return float3(-24,7,13);
}
struct Inputs {};
void compute(ComputeContext c, Inputs i) {
 uint id=c.id.x;
 bool close=distance(translation_load(id),eye(c.time+c.dt))<.9;
 far_scale_store(id,close?float3(0):float3(1));
 near_scale_store(id,close?float3(.1466666667):float3(0));
}
