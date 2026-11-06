// Continuous vertex-gradient reconstruction for shading dense scalar fields.
// Intersection/occupancy keep the exact derivative of the trilinear scalar.
import implicit_grid;

float3 implicit_shading_gradient<G:IImplicitGrid>(G grid,float3 local) {
    let dims=grid.dimensions();
    float3 q=clamp((local+1.)*.5*float3(dims)-.5,0.,float3(dims-1u));
    int3 cell=min(int3(floor(q)),int3(dims)-2);
    float3 f=q-float3(cell);
    float samples[48];float magnitude=0.;
    for(uint corner=0u;corner<8u;corner++) {
        int3 vertex=cell+int3(corner&1u,(corner>>1u)&1u,(corner>>2u)&1u);
        for(uint axis=0u;axis<3u;axis++) {
            int3 delta=int3(0);delta[axis]=1;
            float a=grid.value(uint3(max(vertex-delta,int3(0))));
            float b=grid.value(uint3(min(vertex+delta,int3(dims)-1)));
            samples[corner*6u+axis*2u]=a;samples[corner*6u+axis*2u+1u]=b;
            magnitude=max(magnitude,max(abs(a),abs(b)));
        }
    }
    if(!(magnitude>0.)) return float3(0.);
    float3 gradient=0.;
    // Divide before subtracting to accommodate every finite field scale.
    float3 scale=float3(dims)/float(max(dims.x,max(dims.y,dims.z)));
    for(uint corner=0u;corner<8u;corner++) {
        int3 offset=int3(corner&1u,(corner>>1u)&1u,(corner>>2u)&1u);
        int3 vertex=cell+offset;
        float3 weights=lerp(1.-f,f,float3(offset));
        float weight=weights.x*weights.y*weights.z;
        for(uint axis=0u;axis<3u;axis++) {
            float width=float(min(vertex[axis]+1,int(dims[axis])-1)-max(vertex[axis]-1,0));
            gradient[axis]+=weight*scale[axis]*(samples[corner*6u+axis*2u+1u]/magnitude-samples[corner*6u+axis*2u]/magnitude)/width;
        }
    }
    return gradient;
}
