// Ray-hit surfaces: a hit triangle's corners, read in place from the raster
// vertex, attribute and index buffers (rt::hit_record), become the raster
// Varying and Coordinates, so hits shade through surface_at with the same
// material table and programs as raster fragments.
import vork_constants;
import frame;
import stream;
import material;
import surface;
import variant;

__exported import hit_geometry;
#ifndef VORK_ATTRIBUTES
#define VORK_ATTRIBUTES 0
#endif

// One camera pixel at `world`'s depth: the offset of the neighbouring pixel's
// point, on the plane through `world` facing the camera. `pixel` is one pixel
// in clip units (2/size). Behind the camera the offset only flips sign.
public float3 camera_pixel_offset(float3 world, float4 clip, float2 pixel, uint axis) {
    var shifted=clip;shifted[axis]+=pixel[axis]*clip.w;
    let h=mul(shifted,frame.inverse_vp);
    return h.xyz/h.w-world;
}
// The pixel footprint at a hit (pbrt-v4's camera approximation): the camera's
// pixel spread at the hit, carried along the arriving ray onto the triangle's
// plane. At camera-ray hits this is the raster derivative; secondary hits take
// the footprint a camera ray would have there. Barycentric differentials then
// difference every hit attribute, so programs and textures see one footprint.
Coordinates hit_coordinates(HitTriangle t, float3 b, Varying v, float3 geometric, bool object_front, float3 direction, float2 pixel) {
    let model=object_model(t.object);
    let w0=mul(float4(t.c[0].POSITION,1.),model).xyz;
    let e1=mul(float4(t.c[1].POSITION,1.),model).xyz-w0;let e2=mul(float4(t.c[2].POSITION,1.),model).xyz-w0;
    let n=cross(e1,e2);let area=dot(n,n);
    Coordinates k={};k.incoming=-direction;
    if(abs(v.TANGENT.w)<2.) {k.shadow_offset=hit_shadow_offset(t,b,object_front);}
    if (!(area>0.) || all(pixel==float2(0.))) {return k;}
    let clip=mul(float4(v.world,1.),frame.vp);
    let facing=dot(direction,n);
    let along=direction/(abs(facing)>1e-4*sqrt(area) ? facing : 1e-4*sqrt(area));
    float3 dp[2];float3 db[2];
    for (uint axis=0u;axis<2u;axis++) {
        let offset=camera_pixel_offset(v.world,clip,pixel,axis);
        dp[axis]=offset-along*dot(offset,n);
        let b1=dot(cross(dp[axis],e2),n)/area;let b2=dot(cross(e1,dp[axis]),n)/area;
        db[axis]=float3(-b1-b2,b1,b2);
    }
    // Positions and UVs are linear on the triangle; sockets (programs only) are
    // evaluated at the offset points like the raster quad's neighbours.
    k.position_dx=dp[0];k.position_dy=dp[1];
    let uv1_used=abs(v.TANGENT.w)<2.;
    k.uv_dx=t.c[0].TEXCOORD_0*db[0].x+t.c[1].TEXCOORD_0*db[0].y+t.c[2].TEXCOORD_0*db[0].z;
    k.uv_dy=t.c[0].TEXCOORD_0*db[1].x+t.c[1].TEXCOORD_0*db[1].y+t.c[2].TEXCOORD_0*db[1].z;
    k.uv1_dx=select(uv1_used,t.c[0].TEXCOORD_1*db[0].x+t.c[1].TEXCOORD_1*db[0].y+t.c[2].TEXCOORD_1*db[0].z,float2(0.));
    k.uv1_dy=select(uv1_used,t.c[0].TEXCOORD_1*db[1].x+t.c[1].TEXCOORD_1*db[1].y+t.c[2].TEXCOORD_1*db[1].z,float2(0.));
    let vx=hit_varying(t,b+db[0]);let vy=hit_varying(t,b+db[1]);
#if VORK_ATTRIBUTES > 0
    k.attribute_dx[0]=vx.attr0-v.attr0;k.attribute_dy[0]=vy.attr0-v.attr0;
#endif
#if VORK_ATTRIBUTES > 1
    k.attribute_dx[1]=vx.attr1-v.attr1;k.attribute_dy[1]=vy.attr1-v.attr1;
#endif
#if VORK_ATTRIBUTES > 2
    k.attribute_dx[2]=vx.attr2-v.attr2;k.attribute_dy[2]=vy.attr2-v.attr2;
#endif
#if VORK_ATTRIBUTES > 3
    k.attribute_dx[3]=vx.attr3-v.attr3;k.attribute_dy[3]=vy.attr3-v.attr3;
#endif
#if VORK_ATTRIBUTES > 4
    k.attribute_dx[4]=vx.attr4-v.attr4;k.attribute_dy[4]=vy.attr4-v.attr4;
#endif
#if VORK_ATTRIBUTES > 5
    k.attribute_dx[5]=vx.attr5-v.attr5;k.attribute_dy[5]=vy.attr5-v.attr5;
#endif
#if VORK_ATTRIBUTES > 6
    k.attribute_dx[6]=vx.attr6-v.attr6;k.attribute_dy[6]=vy.attr6-v.attr6;
#endif
#if VORK_ATTRIBUTES > 7
    k.attribute_dx[7]=vx.attr7-v.attr7;k.attribute_dy[7]=vy.attr7-v.attr7;
#endif
    k.rest_dx=vx.rest-v.rest;k.rest_dy=vy.rest-v.rest;
    k.normal_dx=surface_geometric(vx,object_front)-geometric;
    k.normal_dy=surface_geometric(vy,object_front)-geometric;
    return k;
}
public struct HitSurface { public Surface s; public Varying v; public Coordinates k; public float3 face; public bool entering; };
// The surface a ray arriving along `direction` sees at barycentrics `barycentrics`.
public HitSurface hit_surface<V:IVariant, P:IProgram, G:IHitGeometry>(G g,uint instance, uint primitive, uint object, float3x4 world_to_object, float2 barycentrics, float3 direction, float2 pixel) {
    return hit_surface<V,P,G>(g,instance,primitive,object,world_to_object,barycentrics,direction,pixel,Lobe(0u,0.));
}
// `lobe` selects a mixed surface's lobe (surface.slang Lobe).
public HitSurface hit_surface<V:IVariant, P:IProgram, G:IHitGeometry>(G g,uint instance, uint primitive, uint object, float3x4 world_to_object, float2 barycentrics, float3 direction, float2 pixel, Lobe lobe) {
    return triangle_surface<V,P>(hit_triangle<G>(g,instance,primitive,object,world_to_object),primitive,barycentrics,direction,pixel,lobe);
}
// Shared retained point/footprint reconstruction, also used to prove that a
// transparent collection candidate cannot contribute at its shading sample.
public HitSurface triangle_point(HitTriangle t, uint primitive, float2 barycentrics, float3 direction, float2 pixel) {
    let b=float3(1.-barycentrics.x-barycentrics.y,barycentrics);
    var h=(HitSurface)0;
    h.v=hit_varying(t,b);h.v.primitive=primitive+t.primitive_offset;
    h.face=hit_face(t);
    h.entering=dot(h.face,direction)<0.;
    let geometric=surface_geometric(h.v,h.entering);
    h.k=hit_coordinates(t,b,h.v,geometric,h.entering,direction,pixel);
    return h;
}
// The surface of an already loaded triangle `t` (primitive `primitive`) at `barycentrics`.
public HitSurface triangle_surface<V:IVariant, P:IProgram,let decode_table:bool=true>(HitTriangle t, uint primitive, float2 barycentrics, float3 direction, float2 pixel, Lobe lobe, float3 filter=float3(0.)) {
    var h=triangle_point(t,primitive,barycentrics,direction,pixel);
    h.k.material_filter=filter;
    h.s=surface_at<V,P,decode_table>(h.v,h.entering,-direction,h.k,lobe);
    if (!h.entering) {h.face=-h.face;}
    return h;
}
