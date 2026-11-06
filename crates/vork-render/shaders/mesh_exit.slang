// Retained stackless exit trees. Clients provide storage and current vertices;
// topology, bounds traversal and outward triangle acceptance are shared.
interface IExitGeometry {
    uint4 word(uint node, uint field);
    float3 vertex(uint triangle, uint corner);
    bool bounded();
}
struct MeshExit {float distance;float2 barycentric;uint triangle;bool found;};
MeshExit mesh_exit<G:IExitGeometry>(G geometry,uint count,float3 p,float3 d,float maximum,float edge) {
    MeshExit hit={maximum,float2(0.),0u,false};
    var node=0u;
    for (;;) {
        if (node>=count) break;
        let low=geometry.word(node,0u);let high=geometry.word(node,1u);
        if (geometry.bounded()) {
            let raw_lo=asfloat(low.xyz);let raw_hi=asfloat(high.xyz);
            // Triangle edge tolerance also expands the conservative bounds.
            let extent=raw_hi-raw_lo;let scale=max(abs(extent.x),max(abs(extent.y),abs(extent.z)));
            let padding=edge==0. || scale==0. ? 0. : (2.*edge*scale)*length(extent/scale);
            let lo=raw_lo-padding;let hi=raw_hi+padding;
            var near=0.;var far=hit.distance;
            for (uint axis=0u;axis<3u;axis++) {
                if (d[axis]==0.) {
                    if (p[axis]<lo[axis] || p[axis]>hi[axis]) far=-1.;
                } else {
                    let a=(lo[axis]-p[axis])/d[axis];let z=(hi[axis]-p[axis])/d[axis];
                    near=max(near,min(a,z));far=min(far,max(a,z));
                }
            }
            if (far<near) {node=high.w==0xffffffffu ? low.w : node+1u;continue;}
        }
        node++;
        if (high.w==0xffffffffu) continue;
        for (uint offset=0u;offset<low.w;offset++) {
        let triangle=high.w+offset;
        let va=geometry.vertex(triangle,0u);let vb=geometry.vertex(triangle,1u);let vc=geometry.vertex(triangle,2u);
        // Normalize coordinates and direction before cross products: neither
        // underflow on small geometry nor overflow on large geometry is parallelism.
        let magnitude=max(abs(va),max(abs(vb),abs(vc)));
        let scale=max(magnitude.x,max(magnitude.y,magnitude.z));
        let speed=max(abs(d.x),max(abs(d.y),abs(d.z)));
        if (scale==0. || speed==0.) {continue;}
        let a=va/scale;let e1=vb/scale-a;let e2=vc/scale-a;let direction=d/speed;
        let q=cross(direction,e2);let determinant=dot(e1,q);
        if (determinant>=0.) {continue;}
        let t=p/scale-a;let u=dot(t,q)/determinant;
        let r=cross(t,e1);let w=dot(direction,r)/determinant;
        // t_normalized * scale / speed, evaluated with a bounded mantissa.
        // This keeps an intermediate overflow from hiding a representable distance.
        int et,es,ed;
        let mt=frexp(dot(e2,r)/determinant,et);let ms=frexp(scale,es);let md=frexp(speed,ed);
        let distance=ldexp(mt*ms/md,et+es-ed);
        if (u < -edge || w < -edge || u+w>1.+edge || distance<=0. || distance>=hit.distance) continue;
        hit=MeshExit(distance,float2(u,w),triangle,true);
        }
    }
    return hit;
}
