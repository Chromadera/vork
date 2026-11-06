// Direction proposal over an implicit field's conservative transformed domain.
// Bounds guide rays only. The actual trilinear zero set supplies every source
// point, normal, coverage and radiance; no proxy surface contributes energy.
import hit_geometry;
import frame;
import vork_ray_sampling;
import vork_constants;
public struct FieldEmitter {public uint instance;public uint object;public uint descriptor;};
public FieldEmitter field_emitter<G:IHitGeometry>(uint4 table,uint source) {
    let at=table.x+3u*G.word(table.z+source);
    return FieldEmitter(G.word(at),G.word(at+1u),G.word(at+2u));
}
public float field_emitter_cone<G:IHitGeometry>(FieldEmitter source,float3 from,out float3 axis) {
    let model=object_model(source.object);let center=mul(float4(0.,0.,0.,1.),model).xyz;
    let extent=1.-1./float3(G.word(source.descriptor),G.word(source.descriptor+1u),G.word(source.descriptor+2u));
    // Eight domain corners also cover shear from composed nonuniform transforms.
    var squared_radius=0.;
    for(uint corner=0u;corner<8u;corner++) {
        let local=extent*(2.*float3(corner&1u,(corner>>1u)&1u,(corner>>2u)&1u)-1.);
        let delta=mul(float4(local,0.),model).xyz;
        squared_radius=max(squared_radius,dot(delta,delta));
    }
    let delta=center-from;let squared_distance=dot(delta,delta);
    axis=squared_distance>0. ? delta/sqrt(squared_distance) : float3(0.,0.,1.);
    if(squared_distance<=squared_radius) {return 2.;}
    let sine_squared=min(squared_radius/squared_distance,1.);
    // Avoid subtracting nearly equal floats for a distant, small source.
    return sine_squared/(1.+sqrt(1.-sine_squared));
}
public float3 field_emitter_direction<G:IHitGeometry>(FieldEmitter source,float3 from,float2 u,out float pdf) {
    float3 axis;let width=field_emitter_cone<G>(source,from,axis);
    pdf=width>0. ? 1./(2.*REG_PI*width) : 0.;
    let height=u.x*width;let cosine=1.-height;
    let sine=sqrt(max(height*(2.-height),0.));let phi=2.*REG_PI*u.y;
    return mul(float3(sine*cos(phi),sine*sin(phi),cosine),ray_basis(axis));
}
public float field_emitter_pdf<G:IHitGeometry>(uint4 table,uint instance,float3 from) {
    for(uint i=0u;i<table.w;i++) {
        let source=field_emitter<G>(table,i);
        if(source.instance!=instance) {continue;}
        float3 axis;let width=field_emitter_cone<G>(source,from,axis);
        return width>0. ? 1./(float(table.w)*2.*REG_PI*width) : 0.;
    }
    return 0.;
}
