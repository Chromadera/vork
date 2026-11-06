// No fibre surface in precompiled scenes. Program scenes link the shared std evaluator.
public float3 strand_direct(float4 a,float4 b,float3 t,float3 n,float3 wo,float3 wi,out float pdf) {pdf=0.;return float3(0.);}
public float3 strand_shadow(float3 p,float3 n,float3 d,uint base,uint strand) {return float3(0.);}
public float3 strand_world(float4 a,float4 b,float3 t,float3 n,float3 wo,float3 world) {return float3(0.);}
public float3 strand_sample(float4 a,float4 b,float3 t,float3 n,float3 wo,float3 u,out float3 direction,out bool reflected,out float pdf) {direction=float3(0.);reflected=true;pdf=0.;return float3(0.);}

public float3 strand_neighbours(float4 a,float4 b,float3 t,float3 wo,float3 wi,float behind) {return float3(0.);}
public float3 strand_forward(float4 a,float4 b,float3 tangent,float3 incident) {return float3(1.);}

public float3 strand_forward_at(float4 a,float4 b,float3 tangent,float3 normal,float3 outgoing) {return float3(1.);}
