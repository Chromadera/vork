// 64-pixel tiles, 24 logarithmic view-depth slices. Each invocation owns a
// complete counted list; capacity is the installed light count, with no atomics.
import vork_constants;
import vork_prelude;

struct ClusterCullConfig { uint4 grid; uint4 screen; };
[[vk::binding(0, 0)]] ConstantBuffer<Frame> cull_frame : register(b0);
[[vk::binding(1, 0)]] StructuredBuffer<Light> cull_lights : register(t1);
[[vk::binding(2, 0)]] ConstantBuffer<ClusterCullConfig> cull_config : register(b2);
[[vk::binding(3, 0)]] RWStructuredBuffer<uint> lists : register(u3);

[shader("compute")]
[numthreads(REG_CLUSTER_GROUP, REG_CLUSTER_GROUP, 1)]
void cull(uint3 id : SV_DispatchThreadID) {
    if (any(id.xy>=cull_config.grid.xy)) { return; }
    let near=cull_frame.params.x;let far=cull_frame.params.y;
    let z0=near*pow(far/near,float(id.z)/float(cull_config.screen.z));
    let z1=near*pow(far/near,float(id.z+1u)/float(cull_config.screen.z));
    let pixel0=float2(id.xy*cull_config.screen.w);
    let pixel1=float2(min((id.xy+uint2(1u))*cull_config.screen.w,cull_config.screen.xy));
    var lo=float3(REG_CLUSTER_BOUND);var hi=float3(-REG_CLUSTER_BOUND);
    for(uint corner=0u;corner<4u;corner++) {
        let pixel=float2(((corner&1u)!=0u ? pixel1.x : pixel0.x),((corner&2u)!=0u ? pixel1.y : pixel0.y));
        let ndc=pixel/float2(cull_config.screen.xy)*float2(2.,-2.)+float2(-1.,1.);
        let a=mul(float4(ndc,0.,1.),mul(cull_frame.inverse_vp,cull_frame.view));
        let b=mul(float4(ndc,1.,1.),mul(cull_frame.inverse_vp,cull_frame.view));
        let p=a.xyz/a.w;let q=b.xyz/b.w;
        let p0=p+(q-p)*((-z0-p.z)/(q.z-p.z));
        let p1=p+(q-p)*((-z1-p.z)/(q.z-p.z));
        lo=min(lo,min(p0,p1));hi=max(hi,max(p0,p1));
    }
    // Round outward to cover floating-point reconstruction at tile/slice boundaries.
    lo-=float3(REG_CLUSTER_PADDING);hi+=float3(REG_CLUSTER_PADDING);
    let cluster=(id.z*cull_config.grid.y+id.y)*cull_config.grid.x+id.x;
    let base=cluster*cull_config.grid.z;
    var visible_count=0u;
    // Stable kind sorting keeps neighbouring fragments on the same shader path.
    for(uint kind=0u;kind<REG_LIGHT_KIND_COUNT;kind++) {
        for(uint i=0u;i<uint(frame_light_count(cull_frame));i++) {
            let l=cull_lights[i];
            if (uint(l.position.w)!=kind) { continue; }
            // Animated and imported lights stay installed when their power is
            // zero, but neither surfaces nor media can receive their radiance.
            if (all(l.radiance.rgb==float3(0.))) {continue;}
            var visible=true;
            if (l.position.w>0.) {
                let center=(mul(float4(l.position.xyz,1.),cull_frame.view)).xyz;
                // Shared support includes authored ranges and the explicit
                // integrator light_threshold influence bound. The latter is an
                // intensity approximation, not a bound on glossy or phase peaks.
                if (l.projection.y>0.) {
                    let delta=center-clamp(center,lo,hi);
                    visible=dot(delta,delta)<=l.projection.y*l.projection.y;
                }
                if (visible && (l.position.w==2. || l.position.w==3.)) {
                    let direction=normalize((mul(float4(l.direction.xyz,0.),cull_frame.view)).xyz);
                    let half_extent=(hi-lo)*.5;
                    let offset=(hi+lo)*.5-center;
                    let axial=dot(direction,offset);
                    if (l.position.w==3.) {
                        // Support of the AABB in the emitter normal direction.
                        // Reject only if the complete cluster is behind its plane.
                        visible=axial+dot(abs(direction),half_extent)>=0.;
                    } else if (l.direction.w>=0.) {
                        // The cluster's bounding sphere contains its complete
                        // frustum AABB. Signed cone-side distance is a lower
                        // bound even behind the apex; the apex plane rejects
                        // the remaining fully backwards spheres.
                        let cluster_radius=length(half_extent);
                        let cosine=l.direction.w;
                        let sine=sqrt(max(1.-cosine*cosine,0.));
                        let radial=length(offset-direction*axial);
                        visible=axial>=-cluster_radius && radial*cosine-axial*sine<=cluster_radius;
                    }
                }
            }
            if (visible) {
                lists[base+REG_CLUSTER_LIST_HEADER+visible_count]=i;
                visible_count++;
            }
        }
    }
    lists[base]=visible_count;
}
