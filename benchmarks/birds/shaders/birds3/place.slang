struct Inputs {};
void compute(ComputeContext c, Inputs i) {
    uint id=c.id.x; float3 position=next_p_load(id), velocity=next_v_load(id);
    float3 old=v_load(id);
    p_store(id,position); v_store(id,velocity); translation_store(id,position);
    float3 direction=normalize(velocity);
    float3 axis=cross(float3(0,0,1),direction); float w=1.+direction.z;
    float4 q=float4(axis,w); float norm=length(q);
    q=norm>1e-6 ? q/norm : float4(0,1,0,0);
    // Coordinated turn: lateral acceleration tilts lift into the bend.
    float3 right=normalize(cross(float3(0,1,0),direction)+float3(1e-6,0,0));
    float bank=clamp(-dot((velocity-old)/c.dt,right)/9.81,-.8,.8);
    float s=sin(bank*.5),cs=cos(bank*.5);
    rotation_store(id,float4(q.xyz*cs+float3(q.y,-q.x,q.w)*s,q.w*cs-q.z*s));
}
