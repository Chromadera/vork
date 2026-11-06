import vork;
struct Inputs { Image image; Image lighting_weight; };
float3 environment(EnvironmentContext ctx, Inputs i) {
    float3 d=normalize(ctx.direction);
    float u=dot(d.xz,d.xz)>0 ? atan2(d.z,d.x)/6.283185307179586+0.5 : 0.5;
    float2 uv=float2(u,acos(clamp(d.y,-1.,1.))/3.141592653589793);
    return sample(i.image,uv,ctx).rgb*(ctx.camera ? 1. : sample(i.lighting_weight,uv,ctx).r);
}
