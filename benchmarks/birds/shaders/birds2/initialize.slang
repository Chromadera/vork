struct Inputs {};
uint hash(uint x) { x ^= x >> 16; x *= 0x7feb352du; x ^= x >> 15; x *= 0x846ca68bu; return x ^ (x >> 16); }
float random(uint x) { return float(hash(x) >> 8) / 16777216.; }
void compute(ComputeContext c, Inputs i) {
    uint id=c.id.x;
    float extent=min(12.,4.*pow(float(p_count)/1000.,1./3.));
    float u=random(id*3+c.seed)*2.-1.;
    float angle=random(id*3+1+c.seed)*6.28318530718;
    float radius=sqrt(random(id*3+2+c.seed));
    // Long, curved, uneven ribbon, rather than a filled equilibrium ellipsoid.
    float3 position=float3(u*extent, 2.*sin(u*3.5)+radius*cos(angle)*2.,
                           3.*sin(u*2.5)+radius*sin(angle)*2.5);
    float3 velocity=float3(2.5,1.5*cos(u*3.5),2.*cos(u*2.5));
    p_store(id,position); translation_store(id,position);
    v_store(id,velocity);
    float3 direction=normalize(velocity);
    float4 q=float4(cross(float3(0,0,1),direction),1.+direction.z);
    rotation_store(id,normalize(q));
}
