// Current area/emission mesh proposal over conservative, immutable source support.
// Read current corners through the consumer's existing hit geometry; own no
// geometry buffers, random stream, footprint or surface/emission evaluator.
import hit_geometry;
import vork_constants;
import stream;

public struct EmitterTriangle { public uint instance; public uint object; public uint primitive; public float2 barycentrics; public float probability; };
public uint emitter_mesh_source<G:IHitGeometry>(G g,uint base, uint source) {
    return base+3u+g.word(base+2u)+source*3u;
}
public uint emitter_mesh_leaves(uint count) {return count<=1u ? 1u : (1u<<(firstbithigh(count-1u)+1u));}
// Invert the immutable cumulative triangle counts, independently of weighting.
public EmitterTriangle emitter_mesh_triangle<G:IHitGeometry>(G g,uint base,uint triangle) {
    let count=g.word(base);
    var low=0u;var high=count;
    while (low<high) {let middle=(low+high)/2u;if (triangle<g.word(emitter_mesh_source<G>(g,base,middle)+2u)) {high=middle;} else {low=middle+1u;}}
    let record=emitter_mesh_source<G>(g,base,low);
    EmitterTriangle result={};result.instance=g.word(record);result.object=g.word(record+1u);
    result.primitive=triangle-(low==0u ? 0u : g.word(emitter_mesh_source<G>(g,base,low-1u)+2u));
    return result;
}
// Sibling means cover equal leaf counts. Form their ratio without adding
// potentially large finite masses; both sampling and PDF reconstruction use it.
float emitter_mesh_left_probability(float left,float right) {
    // Native division can form a reciprocal that flushes to zero for large
    // finite denominators. A shared power-of-two scale preserves the ratio
    // while keeping both reciprocals and products in the normal range.
    let exponent=int((asuint(max(left,right))>>23u)&255u)-127;
    let scale=asfloat(uint(127-exponent/2)<<23u);
    left*=scale;right*=scale;
    if(left>=right) {return left>0. ? 1./(1.+right/left) : 0.;}
    let ratio=left/right;return ratio/(1.+ratio);
}
// Use the actual branch probabilities, including FP32 rounding.
float emitter_mesh_weighted_probability<G:IHitGeometry>(G g,uint base,uint instance,uint primitive,StructuredBuffer<float> distribution) {
    if (g.word(base)==0u || g.word(base+3u+instance)==0xffffffffu || distribution[1]<=0.) {return 0.;}
    let source=g.word(base+3u+instance);
    let triangle=primitive+(source==0u ? 0u : g.word(emitter_mesh_source<G>(g,base,source-1u)+2u));
    var node=emitter_mesh_leaves(g.word(base+1u))+triangle;var probability=1.;
    while(node>1u) {
        let parent=node/2u;let left=distribution[2u*parent];let right=distribution[2u*parent+1u];
        if(max(left,right)<=0.) {return 0.;}
        let p=emitter_mesh_left_probability(left,right);probability*=select((node&1u)!=0u,1.-p,p);node=parent;
    }
    return probability;
}
// Defensive mixture: half current emitting-area proposal, half uniform over
// immutable source triangles. The latter is independent of radiance, area and
// tree rounding, so even an HDR sibling that owns no weighted interval retains
// support. This is a normalized proposal, not a probability or radiance clamp.
public float emitter_mesh_probability<G:IHitGeometry>(G g,uint base,uint instance,uint primitive,StructuredBuffer<float> distribution) {
    if (g.word(base)==0u || g.word(base+3u+instance)==0xffffffffu || distribution[1]<=0.) {return 0.;}
    return (1.-REG_MESH_SOURCE_UNIFORM_MIX)*emitter_mesh_weighted_probability<G>(g,base,instance,primitive,distribution)
        +REG_MESH_SOURCE_UNIFORM_MIX/float(g.word(base+1u));
}
// Requires nonempty support. The geometry and distribution refer to this slot's current pose.
public EmitterTriangle emitter_mesh_sample<G:IHitGeometry>(G g,uint base,float3 u,StructuredBuffer<float> distribution) {
    let leaves=emitter_mesh_leaves(g.word(base+1u));var node=1u;
    var choice=min(u.x,asfloat(0x3f7fffffu));
    if(distribution[1]<=0.) {return emitter_mesh_triangle<G>(g,base,0u);}
    // Keep every bit of the triangle coordinate. Splitting choice itself
    // would halve its cardinality and drop alternating leaves of a balanced
    // tree at the sampler's resolution. Split an area coordinate instead;
    // its conditional residual is still uniform and independent of choice.
    let uniform=u.z<REG_MESH_SOURCE_UNIFORM_MIX;
    let point=(u.z-(uniform ? 0. : REG_MESH_SOURCE_UNIFORM_MIX))
        /(uniform ? REG_MESH_SOURCE_UNIFORM_MIX : 1.-REG_MESH_SOURCE_UNIFORM_MIX);
    if(uniform) {
        // Uniform selection never recovers a tiny weighted mass by
        // subtracting it from one.
        node=leaves+min(uint(choice*float(g.word(base+1u))),g.word(base+1u)-1u);
    } else {
        while(node<leaves) {
            let left=distribution[2u*node];let right=distribution[2u*node+1u];let p=emitter_mesh_left_probability(left,right);
            if(choice<p) {choice/=p;node*=2u;}
            else {choice=(choice-p)/(1.-p);node=2u*node+1u;}
            choice=min(choice,asfloat(0x3f7fffffu));
        }
    }
    var result=emitter_mesh_triangle<G>(g,base,node-leaves);
    result.probability=emitter_mesh_probability<G>(g,base,result.instance,result.primitive,distribution);
    let radius=sqrt(u.y);result.barycentrics=float2(radius*(1.-point),0.);result.barycentrics.y=radius-result.barycentrics.x;
    return result;
}
// Caller supplies the current world-space triangle area, never rest-pose area.
public float emitter_mesh_pdf<G:IHitGeometry>(G g,uint base, uint instance, float area, float3 from, float3 point, float3 face) {
    if (g.word(base)==0u || g.word(base+3u+instance)==0xffffffffu) {return 0.;}
    let delta=point-from;let squared=dot(delta,delta);
    let cosine=abs(dot(face,normalize(delta)));
    return area>0. && cosine>0. ? squared/(area*cosine) : 0.;
}

