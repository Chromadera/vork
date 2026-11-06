// Source-domain radiance over the shared geometric proposal.
import hit;
import vork_constants;
import stream;
import variant;
import bsdf;
__exported import emitter_mesh_proposal;
import surface;

// The surface closure is the sole physical emission definition. Light samples
// carry zero differentials; hits retain their footprint. Lit is never queried.
float3 emitter_mesh_radiance<V:IVariant,P:IProgram>(HitSurface source) {
    var first=source.s;
    if(P.lobes || (P.lit && P.lit_material(source.v))) {
        first=surface_at<V,P>(source.v,source.entering,source.s.view,source.k,Lobe(0u,0.));
    }
    if(first.discarded || first.is_hair) {return float3(0.);}
    var radiance=float3(0.);
    for(uint i=0u;i<max(first.lobes,1u);i++) {
        var s=first;
        if(i>0u) {s=surface_at<V,P>(source.v,source.entering,source.s.view,source.k,Lobe(i,0.));}
        radiance+=surface_emission(s.closure,layers<V>(s.closure,source.s.view))*s.weight;
    }
    return radiance;
}
