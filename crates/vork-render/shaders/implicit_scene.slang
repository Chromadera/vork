// Dense fields read the same retained pages that simulation writes. Descriptors
// contain spans, so grids can cross storage pages without a second allocation.
import implicit_grid;
import implicit_normal;
import vork_constants;
import hit;
import variant;
import surface;
import frame;
import material;
import stream;
struct SceneImplicitGrid<G:IHitGeometry> : IImplicitGrid {
    G g;
    uint descriptor;
    uint3 dimensions() {return uint3(g.word(descriptor),g.word(descriptor+1u),g.word(descriptor+2u));}
    uint cell_step(uint3 cell) {
        let d=dimensions();let blocks=(d-2u+REG_IMPLICIT_BLOCK)/REG_IMPLICIT_BLOCK;let b=cell/REG_IMPLICIT_BLOCK;
        let slot=g.word(descriptor+4u+4u*g.word(descriptor+3u));
        let signs=g.load(slot,b.x+blocks.x*(b.y+blocks.y*b.z));
        return signs==1u || signs==2u ? REG_IMPLICIT_BLOCK : 1u;
    }
    float value(uint3 vertex) {
        let d=dimensions();let index=vertex.x+d.x*(vertex.y+d.y*vertex.z);
        let count=g.word(descriptor+3u);
        for(uint i=0u;i<count;i++) {
            let at=descriptor+4u+i*4u;let first=g.word(at+2u);
            if(index>=first && index-first<g.word(at+3u)) return asfloat(g.load(g.word(at),g.word(at+1u)+index-first));
        }
        return 0.;
    }
};
struct FieldHit {bool found;float t;float3 local;float3 normal;float3 shading_normal;};
#ifdef VORK_FIELD_OUTLINE
// Bound Slang's graph across visibility, shadows and secondary transport.
// Camera kernels keep this boundary through the driver; other ray kernels
// restore driver inlining freedom, as for path helpers.
[noinline]
#endif
FieldHit field_intersection<G:IHitGeometry>(G g,uint descriptor,uint object,float3 origin,float3 direction,float minimum,float maximum) {
#if defined(VORK_CAMERA_FUSED) || defined(VORK_NO_IMPLICIT)
    // Triangle-only scenes never query a field. Keep unreachable traversal
    // out of the driver compiler, including retained classification kernels.
    FieldHit absent={};return absent;
#else
    let inverse=(float3x4)object_normal(object);
    let p=mul(inverse,float4(origin,1.));let d=mul(inverse,float4(direction,0.));
    SceneImplicitGrid<G> grid;grid.g=g;grid.descriptor=descriptor;
    let root=implicit_grid_hit(grid,0.,p,d,minimum,maximum);
    FieldHit h={};h.found=root.found && root.valid;h.t=root.distance;h.local=p+d*h.t;
    if(!root.valid || (root.found && (!all(isfinite(root.gradient)) || all(root.gradient==float3(0.))))) {
        InterlockedMax(shader_errors[0],REG_IMPLICIT_INVALID);InterlockedMin(shader_errors[1],object);h.found=false;
    }
    h.normal=h.found ? normalize(mul(root.gradient,(float3x3)inverse)) : float3(0.,1.,0.);
    h.shading_normal=h.normal;
    if(h.found && !root.boundary) {
        float3 gradient=implicit_shading_gradient(grid,h.local);
        if(all(isfinite(gradient)) && any(gradient!=float3(0.))) {
            float3 smooth=normalize(mul(gradient,(float3x3)inverse));
            if(dot(smooth,h.normal)>0.) h.shading_normal=smooth;
        }
    }
    return h;
#endif
}
// Raster keys reserve the high primitive bit for a camera-ray crossing ordinal.
// Triangle index buffers cannot address this many three-index primitives.
FieldHit field_crossing<G:IHitGeometry>(G g,uint descriptor,uint object,uint primitive,float3 origin,float3 direction,float minimum,float maximum) {
    var hit=field_intersection<G>(g,descriptor,object,origin,direction,minimum,maximum);
    for(uint crossing=0u;hit.found && crossing<(~primitive);crossing++) {
        minimum=hit.t+REG_FP32_GAMMA7*max(1.,abs(hit.t));
        hit=field_intersection<G>(g,descriptor,object,origin,direction,minimum,maximum);
    }
    return hit;
}
#ifdef VORK_FIELD_OUTLINE
[noinline]
#endif
HitSurface field_surface<V:IVariant,P:IProgram>(uint object,FieldHit hit,float3 direction,float2 pixel,Lobe lobe) {
    HitSurface h={};
    h.v.object=object;h.v.rest=hit.local;
    h.v.world=mul(float4(hit.local,1.),object_model(object)).xyz;
    h.v.NORMAL=hit.shading_normal;h.v.COLOR_0=float4(1.);h.v.position=float4(0.,0.,0.,1.);
    h.face=hit.normal;h.entering=dot(direction,h.face)<0.;
    h.k.incoming=-direction;
    let clip=mul(float4(h.v.world,1.),frame.vp);
    let denominator=dot(direction,h.face);
    if(abs(denominator)>1e-6) {
        let dx=camera_pixel_offset(h.v.world,clip,pixel,0u);let dy=camera_pixel_offset(h.v.world,clip,pixel,1u);
        h.k.position_dx=dx-direction*(dot(dx,h.face)/denominator);
        h.k.position_dy=dy-direction*(dot(dy,h.face)/denominator);
        let inverse=(float3x3)object_normal(object);
        h.k.rest_dx=mul(inverse,h.k.position_dx);h.k.rest_dy=mul(inverse,h.k.position_dy);
    }
    h.s=surface_at<V,P>(h.v,h.entering,-direction,h.k,lobe);
    if(!h.entering) h.face=-h.face;
    return h;
}
