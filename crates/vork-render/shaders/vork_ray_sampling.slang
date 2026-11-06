import orthonormal_basis;
import vork_constants;
import vork_sampling;
import vork_prelude;
// The consuming pass exports this accessor for its own frame binding.
internal extern float3 ray_eye();
// AO ray sampling (its own sample count), plus the basis and origin helpers shadow rays share.
// Shirley-Chiu concentric map from the unit square to the unit disk.
public float2 ray_concentric_disk(float2 u) {
    let a=2.*u-1.;
    if (all(a==float2(0.))) {return float2(0.);}
    if (abs(a.x)>abs(a.y)) {let phi=REG_PI*.25*a.y/a.x;return a.x*float2(cos(phi),sin(phi));}
    let phi=REG_PI*.5-REG_PI*.25*a.x/a.y;return a.y*float2(cos(phi),sin(phi));
}
// Duff et al. 2017 orthonormal basis; rows store the WGSL columns.
public float3x3 ray_basis(float3 n) {return orthonormal_basis(n);}
// The caller supplies uncertainty from its geometry or screen reconstruction.
public float3 ray_origin(float3 world,float3 geometric,float3 direction,float3 error) {
    return surface_ray_origin(world,error,geometric,direction);
}

public void ray_basis(float3 n,out float3 tangent,out float3 bitangent) {let basis=ray_basis(n);tangent=basis[0];bitangent=basis[1];}
