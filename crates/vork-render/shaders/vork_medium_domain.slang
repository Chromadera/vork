#ifndef VORK_MEDIUM_WORLD_ONLY
#define VORK_MEDIUM_WORLD_ONLY 0
#endif
// One physical domain for raster and sampled consumers. World distances survive
// the inverse object transform. Nonzero signed forward winding means occupied;
// reversing all faces preserves membership, disconnected shells remain additive.
import vork_constants;
import vork_prelude;
import vork_medium_point;
import frame;
internal extern Frame medium_frame();
// The ray parameter remains metres in world space, including under nonuniform scale.
public float3 volume_vec3(uint at) {return float3(asfloat(medium_word(at)),asfloat(medium_word(at+1u)),asfloat(medium_word(at+2u)));}
#ifndef VORK_MEDIUM_BOUNDARIES
#define VORK_MEDIUM_BOUNDARIES 0
#endif
#if VORK_MEDIUM_BOUNDARIES > 0
// One ordered interval per boundary, private to a camera-column invocation.
// Intervals stream without a crossing array or a fixed crossing capacity.
static bool volume_column=false;
static float3 volume_column_origin;
static float volume_column_offset=0.;
public void medium_column_offset(float offset) {volume_column_offset=offset;}
static float3 volume_intervals[VORK_MEDIUM_BOUNDARIES];
public export void medium_column_end() {volume_column=false;}
public export void medium_column_begin(float3 origin) {
    volume_column=true;volume_column_origin=origin;
    for(uint i=0u;i<VORK_MEDIUM_BOUNDARIES;i++) {volume_intervals[i]=float3(-1.,-1.,0.);}
}
#endif
// The next ray distance a boundary query distinguishes from `start`. A column
// resolves distances after adding its offset, so a boundary within one float
// there rounds back to `start`; interval walks advance at least this far.
public float volume_interval_successor(float start) {
    // Never a denormal: Metal flushes them to zero.
    var next=max(screen_next_float(start,true),asfloat(0x00800000u));
#if VORK_MEDIUM_BOUNDARIES > 0
    if(volume_column) {next=max(next,screen_next_float(start+volume_column_offset,true)-volume_column_offset);}
#endif
    return next;
}
// Slabs handle parallel axes explicitly: 0*infinity must not discard a ray
// lying on a node face. Traverse to infinity because winding includes all future
// crossings, even those beyond the integration endpoint.
bool volume_node_hit(float3 lo,float3 hi,float3 o,float3 d,float after) {
    var near=after;var far=REG_FP32_MAX;
    for(uint axis=0u;axis<3u;axis++) {
        if(d[axis]==0.) {if(o[axis]<lo[axis] || o[axis]>hi[axis]) {return false;}}
        else {
            let a=(lo[axis]-o[axis])/d[axis];let b=(hi[axis]-o[axis])/d[axis];
            // Enlarge the far slab for subtraction/division rounding. Extra
            // candidates are safe; losing a grazing face changes winding.
            near=max(near,min(a,b));far=min(far,max(a,b)*(1.+6.*REG_FP32_EPSILON));
        }
    }
    return far>=near;
}
// Signed area of edge (p,q) about the sheared ray, always evaluated with its
// endpoints in one canonical order: both triangles sharing the edge compute the
// same rounded value with opposite signs, whatever the compiler contracts.
// An exactly-zero edge belongs to the triangle traversing it in canonical order.
float volume_edge(float2 p,float2 q,float facing) {
    let forward=p.x<q.x || (p.x==q.x && p.y<q.y);
    let lo=forward ? p : q;let hi=forward ? q : p;
    let area=lo.x*hi.y-lo.y*hi.x;
    return area!=0. ? (forward ? area : -area) : (forward ? facing : -facing);
}
// Watertight crossing test (Woop, Benthin and Wald 2013): a ray through an edge
// shared by two consistently wound triangles crosses exactly one of them, so
// winding never loses or doubles a crossing. Vertices are sheared into the ray
// frame, where the ray is the origin; the test is in that 2D projection.
bool volume_crosses(float3 a,float3 b,float3 c,float3 o,float3 d,float facing) {
    let z=abs(d.x)>abs(d.y) ? (abs(d.x)>abs(d.z) ? 0u : 2u) : (abs(d.y)>abs(d.z) ? 1u : 2u);
    var x=(z+1u)%3u;var y=(x+1u)%3u;
    if(d[z]<0.) {let swap=x;x=y;y=swap;}
    let shear=float2(d[x],d[y])/d[z];
    float3 vertices[3]={a-o,b-o,c-o};float2 projected[3];
    for(uint i=0u;i<3u;i++) {let v=vertices[i];projected[i]=float2(v[x],v[y])-shear*v[z];}
    let u=volume_edge(projected[1],projected[2],facing);
    let v=volume_edge(projected[2],projected[0],facing);
    let w=volume_edge(projected[0],projected[1],facing);
    return (u>0. && v>0. && w>0.) || (u<0. && v<0. && w<0.);
}
public float2 volume_boundary(uint at,float3 o,float3 d,float after,float limit,bool camera_ray=true) {
    let base=medium_frame().medium.x;
    let object=objects[medium_word(at)];
    if ((camera_ray && (object.metadata.y&REG_RAY_CAMERA)==0u) || (object.metadata.y&REG_RAY_VOLUME)==0u) {return float2(limit,0.);}
    var offset=0.;
#if VORK_MEDIUM_BOUNDARIES > 0
    let cached=volume_column && camera_ray;
    let slot=(at-base-medium_word(base))/REG_MEDIUM_BOUND_WORDS;
    if(cached) {
        offset=volume_column_offset;after+=offset;o=volume_column_origin;
        let interval=volume_intervals[slot];
        if(after>=interval.x && after<interval.y) {return float2(min(limit,interval.y-offset),interval.z);}
    }
#endif
    let inverse=transpose(object_normal(medium_word(at)));
    let origin=(mul(float4(o,1.),inverse)).xyz;let direction=(mul(float4(d,0.),inverse)).xyz;
    var nearest=REG_FP32_MAX;var winding=0.;
    var node=base+medium_word(at+1u);let end=base+medium_word(at+2u);
    while(node<end) {
        let escape=base+medium_word(node+3u);
        if(!volume_node_hit(volume_vec3(node),volume_vec3(node+4u),origin,direction,after)) {node=escape;continue;}
        let count=medium_word(node+7u);
        if(count==0u) {node+=REG_MEDIUM_BVH_WORDS;continue;}
        for(var i=0u;i<count;i++) {
            let triangle=node+REG_MEDIUM_BVH_WORDS+i*REG_MEDIUM_TRIANGLE_WORDS;
            let a=volume_vec3(triangle);let b=volume_vec3(triangle+3u);let c=volume_vec3(triangle+6u);
            let edge1=b-a;let edge2=c-a;
            let cross_direction=cross(direction,edge2);let determinant=dot(edge1,cross_direction);
            if (abs(determinant)<=REG_POSITIVE_EPSILON) {continue;}
            if (!volume_crosses(a,b,c,origin,direction,sign(determinant))) {continue;}
            let delta=origin-a;let q=cross(delta,edge1);
            let t=dot(edge2,q)/determinant;
            if (t>after) {nearest=min(nearest,t);winding+=sign(determinant);}
        }
        node=escape;
    }
#if VORK_MEDIUM_BOUNDARIES > 0
    if(cached) {volume_intervals[slot]=float3(after,nearest,winding);}
#endif
    return float2(min(limit,nearest-offset),winding);
}
// Refraction has already found the next exit from its current solid. This
// interval is inside that boundary; only overlapping boundaries need tracing.
public float2 volume_interval_boundary(uint at,float3 o,float3 d,float after,float limit,uint inside_object) {
    if(medium_word(at)==inside_object) {
        let object=objects[inside_object];
        if((object.metadata.y&REG_RAY_CAMERA)==0u || (object.metadata.y&REG_RAY_VOLUME)==0u) {return float2(limit,0.);}
        return float2(limit,1.);
    }
    return volume_boundary(at,o,d,after,limit);
}

// A world is the final, identity-space instance in the same packed table.
// Boundary descriptors contain geometry and a program binding, never transport.

MediumInstance volume_instance_binding(uint index,bool occupied) {
    let base=medium_frame().medium.x;let count=volume_domain_count();
    MediumInstance instance={};instance.program=0xffffffffu;instance.object=0xffffffffu;
    instance.context.inverse=float4x4(float4(1.,0.,0.,0.),float4(0.,1.,0.,0.),float4(0.,0.,1.,0.),float4(0.,0.,0.,1.));
    if(index==count) {
        if((medium_frame().medium.w&1u)!=0u) {
            instance.program=medium_word(base+2u);instance.context.input_base=base+medium_word(base+3u);
        }
    }
#if !VORK_MEDIUM_WORLD_ONLY
    else {
        let at=base+medium_word(base)+index*REG_MEDIUM_BOUND_WORDS;
        if(occupied) {
            instance.program=medium_word(at+3u);instance.object=medium_word(at);
            instance.context.input_base=base+medium_word(at+4u);
            instance.context.inverse=transpose(object_normal(instance.object));
        }
    }
#endif
    return instance;
}
public MediumInstance volume_instance(uint index,float3 o,float3 d,float after,float limit,uint inside_object,bool camera_ray=true) {
    var occupied=true;
#if !VORK_MEDIUM_WORLD_ONLY
    if(index<volume_domain_count()) {
        let base=medium_frame().medium.x;let at=base+medium_word(base)+index*REG_MEDIUM_BOUND_WORDS;
        let boundary=camera_ray ? volume_interval_boundary(at,o,d,after,limit,inside_object) : volume_boundary(at,o,d,after,limit,false);
        occupied=boundary.y!=0.;
    }
#endif
    return volume_instance_binding(index,occupied);
}
public float volume_interval_end(float3 o,float3 d,float after,float limit,uint inside_object,bool camera_ray=true) {
    let base=medium_frame().medium.x;let count=volume_domain_count();var end=limit;
    for(uint i=0u;i<count;i++) {
        let at=base+medium_word(base)+i*REG_MEDIUM_BOUND_WORDS;
        let boundary=camera_ray ? volume_interval_boundary(at,o,d,after,limit,inside_object) : volume_boundary(at,o,d,after,limit,false);
        end=min(end,boundary.x);
    }
    return end;
}

// An empty boundary table is a structural proof, not another transport model.
// Specialization keeps world-only shaders free of unreachable BVH traversal.
public uint volume_domain_count() {
#if VORK_MEDIUM_WORLD_ONLY
    return 0u;
#else
    return medium_word(medium_frame().medium.x+1u);
#endif
}

#ifndef VORK_MEDIUM_DOMAINS
#define VORK_MEDIUM_DOMAINS 0
#endif
// The next crossing and forward winding come from the same boundary query.
// Retain one occupancy bit per domain for this interval, rather than walking
// each BVH again for extinction and again for source integration. This state
// belongs to the caller: nested light connections cannot overwrite it.
public struct MediumBoundaryInterval {
    public float end;
#if VORK_MEDIUM_DOMAINS > 0
    uint occupied[(VORK_MEDIUM_DOMAINS+31)/32];
#endif
};
public MediumBoundaryInterval volume_boundary_interval(float3 o,float3 d,float after,float limit,uint inside_object,bool camera_ray) {
    MediumBoundaryInterval result={};result.end=limit;
    let base=medium_frame().medium.x;let count=volume_domain_count();
    for(uint i=0u;i<count;i++) {
        let at=base+medium_word(base)+i*REG_MEDIUM_BOUND_WORDS;
        let boundary=camera_ray ? volume_interval_boundary(at,o,d,after,limit,inside_object) : volume_boundary(at,o,d,after,limit,false);
        result.end=min(result.end,boundary.x);
#if VORK_MEDIUM_DOMAINS > 0
        if(boundary.y!=0.) {result.occupied[i/32u]|=1u<<(i%32u);}
#endif
    }
    result.end=max(result.end,after);
    return result;
}
public MediumInstance volume_interval_instance(uint index,MediumBoundaryInterval interval,float3 o,float3 d,float midpoint,float limit,uint inside_object,bool camera_ray) {
#if VORK_MEDIUM_DOMAINS > 0
    var occupied=true;
    if(index<volume_domain_count()) {occupied=(interval.occupied[index/32u]&(1u<<(index%32u)))!=0u;}
    return volume_instance_binding(index,occupied);
#else
    return volume_instance(index,o,d,midpoint,limit,inside_object,camera_ray);
#endif
}
