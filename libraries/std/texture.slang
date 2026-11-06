// Footprint filtering and tangent-frame reconstruction shared by imported and authored materials.
import vork;
__exported import std.normal_filter;

public struct TextureFootprint {
    public float2 uv;
    public float2 dx;
    public float2 dy;
    public float2 spread0;
    public float2 spread1;
};
public TextureFootprint texture_footprint(float2 uv, float2 dx, float2 dy, ShadingContext ctx) {
    return TextureFootprint(uv, dx, dy, sqrt(2.)*(dx*ctx.sample_spread.x+dy*ctx.sample_spread.y), sqrt(2.)*dy*ctx.sample_spread.z);
}
bool filtered(TextureFootprint f) {return any(f.spread0!=float2(0.)) || any(f.spread1!=float2(0.));}
float4 texture_one(Image image, TextureFootprint f, ShadingContext ctx) {
    ctx.duv_dx[0]=f.dx;ctx.duv_dy[0]=f.dy;
    return sample(image,f.uv,ctx);
}
TextureFootprint subtap(TextureFootprint f,uint index) {
    f.uv+=select(index<2u,f.spread0,f.spread1)*select((index&1u)==0u,1.,-1.);
    return f;
}
public float4 texture_sample(Image image, TextureFootprint f, ShadingContext ctx) {
    if (!filtered(f)) return texture_one(image,f,ctx);
    return ((texture_one(image,subtap(f,0u),ctx)+texture_one(image,subtap(f,1u),ctx))
           +(texture_one(image,subtap(f,2u),ctx)+texture_one(image,subtap(f,3u),ctx)))*.25;
}
bool tangent_frame(ShadingContext ctx,float3 normal,TextureFootprint f,bool derived,out float3 tangent,out float3 axis,out float handedness) {
    tangent=ctx.TANGENT*select(ctx.front_facing,1.,-1.);handedness=ctx.tangent_sign;var valid=true;
    if (derived || !ctx.has_tangent) {
        let determinant=f.dx.x*f.dy.y-f.dx.y*f.dy.x;
        if(abs(determinant)<1e-12) valid=false;
        else {
            tangent=(ctx.position_dx*f.dy.y-ctx.position_dy*f.dx.y)/determinant;
            let bitangent=(ctx.position_dy*f.dx.x-ctx.position_dx*f.dy.x)/determinant;
            handedness=sign(dot(cross(normal,tangent),bitangent));
        }
    }
    let projected=tangent-normal*dot(normal,tangent);
    valid=valid && !(dot(projected,projected)<1e-12);
    axis=valid ? normalize(projected) : float3(0.);
    return valid;
}
public float3 texture_normal(Image image,TextureFootprint f,ShadingContext ctx,float3 factors,bool derived,out float2 variance) {
    variance=float2(0.);let normal=ctx.NORMAL;
    float3 tangent;float3 axis;float handedness;
    if (!tangent_frame(ctx,normal,f,derived,tangent,axis,handedness)) return normal;
    float3 local;
    if(filtered(f)) {
        let n0=normal_map_unit((texture_one(image,subtap(f,0u),ctx).xyz*2.-1.)*factors);
        let n1=normal_map_unit((texture_one(image,subtap(f,1u),ctx).xyz*2.-1.)*factors);
        let n2=normal_map_unit((texture_one(image,subtap(f,2u),ctx).xyz*2.-1.)*factors);
        let n3=normal_map_unit((texture_one(image,subtap(f,3u),ctx).xyz*2.-1.)*factors);
        local=filtered_normal(n0,n1,n2,n3,variance);
    } else {local=(texture_one(image,f,ctx).xyz*2.-1.)*factors;}
    let world=axis*local.x+cross(normal,axis)*handedness*local.y+normal*local.z;
    if(all(factors==float3(0.,0.,1.))) {variance=float2(0.);return normal;}
    return dot(world,world)>0. ? normalize(world) : normal;
}
public float3 texture_tangent(Image image,TextureFootprint f,ShadingContext ctx,float3 factors,bool derived) {
    float3 tangent;float3 axis;float handedness;
    if (!tangent_frame(ctx,ctx.NORMAL,f,derived,tangent,axis,handedness)) return tangent;
    let local=(texture_sample(image,f,ctx).xyz*2.-1.)*factors;
    return axis*local.x+cross(ctx.NORMAL,axis)*handedness*local.y+ctx.NORMAL*local.z;
}
