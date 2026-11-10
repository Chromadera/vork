// Internal trilinear-cell intersection. Storage, traversal and materials belong
// to the caller. Corner bit order is x + 2*y + 4*z. The values are finite,
// threshold-subtracted and scaled by a common positive magnitude by the caller.
import vork_constants;

struct ImplicitCell {
    float values[8];
};
struct ImplicitCellHit {
    bool found;
    float fraction;
    float3 gradient;
};

float implicit_value(ImplicitCell cell, float3 p) {
    return lerp(lerp(lerp(cell.values[0],cell.values[1],p.x),
                     lerp(cell.values[2],cell.values[3],p.x),p.y),
                lerp(lerp(cell.values[4],cell.values[5],p.x),
                     lerp(cell.values[6],cell.values[7],p.x),p.y),p.z);
}

// Derivative with respect to cell coordinates; transform as a covector to
// obtain a world normal. This is the derivative of the interpolant itself.
float3 implicit_gradient(ImplicitCell cell, float3 p) {
    float3 g;
    g.x=lerp(lerp(cell.values[1]-cell.values[0],cell.values[3]-cell.values[2],p.y),
             lerp(cell.values[5]-cell.values[4],cell.values[7]-cell.values[6],p.y),p.z);
    g.y=lerp(lerp(cell.values[2]-cell.values[0],cell.values[3]-cell.values[1],p.x),
             lerp(cell.values[6]-cell.values[4],cell.values[7]-cell.values[5],p.x),p.z);
    g.z=lerp(lerp(cell.values[4]-cell.values[0],cell.values[5]-cell.values[1],p.x),
             lerp(cell.values[6]-cell.values[2],cell.values[7]-cell.values[3],p.x),p.y);
    return g;
}

// Trilinear interpolation restricted to a straight segment is a cubic.
// Expand its eight basis products; the ray need not be axis aligned.
float4 implicit_polynomial(ImplicitCell cell,float3 origin,float3 direction) {
    float4 c=0.;
    [unroll] for(uint i=0u;i<8u;i++) {
        float3 a,b;
        [unroll] for(uint axis=0u;axis<3u;axis++) {
            bool high=(i&(1u<<axis))!=0u;
            a[axis]=high ? origin[axis] : 1.-origin[axis];
            b[axis]=high ? direction[axis] : -direction[axis];
        }
        c+=cell.values[i]*float4(a.x*a.y*a.z,
            b.x*a.y*a.z+a.x*b.y*a.z+a.x*a.y*b.z,
            b.x*b.y*a.z+b.x*a.y*b.z+a.x*b.y*b.z,b.x*b.y*b.z);
    }
    return c;
}

// Sorted interior stationary points partition the cubic into monotone pieces.
// Using endpoint signs alone would miss two crossings within the same cell.
uint implicit_stationary(float4 c,out float2 roots) {
    roots=0.;
    float a=3.*c.w,b=2.*c.z,d=c.y;
    float magnitude=max(abs(a),max(abs(b),abs(d)));
    if(magnitude==0.) return 0u;
    a/=magnitude;b/=magnitude;d/=magnitude;
    if(a==0.) {
        if(b==0.) return 0u;
        float r=-d/b;
        if(r>0. && r<1.) {roots.x=r;return 1u;}
        return 0u;
    }
    float discriminant=b*b-4.*a*d;
    if(discriminant<0.) return 0u;
    float q=-.5*(b+(b<0. ? -sqrt(discriminant) : sqrt(discriminant)));
    float2 r=q==0. ? float2(-b/(2.*a)) : float2(q/a,d/q);
    if(r.x>r.y) r=r.yx;
    uint count=0u;
    if(r.x>0. && r.x<1.) roots[count++]=r.x;
    if(r.y>0. && r.y<1. && r.y!=r.x) roots[count++]=r.y;
    return count;
}

// Visit ordered zero events lazily. Rounded stationary/end nodes can expose
// four events, but a caller accepting the first must not solve all later roots.
struct ImplicitCellRoots {
    float2 stationary;
    float low;
    float vlow;
    uint count;
    uint interval;
    bool first;
};
ImplicitCellRoots implicit_cell_roots(ImplicitCell cell,float3 start,float3 end) {
    ImplicitCellRoots roots={};roots.first=true;
    float3 direction=end-start;
    float4 c=implicit_polynomial(cell,start,direction);
    if(all(c==float4(0.))) {roots.interval=1u;return roots;}
    roots.count=implicit_stationary(c,roots.stationary);
    roots.vlow=implicit_value(cell,start);
    return roots;
}
bool implicit_cell_next(ImplicitCell cell,float3 start,float3 end,inout ImplicitCellRoots roots,out float fraction) {
    fraction=0.;
    if(roots.interval>roots.count) return false;
    if(roots.first) {
        roots.first=false;
        if(roots.vlow==0.) return true;
    }
    float3 direction=end-start;
    while(roots.interval<=roots.count) {
        float low=roots.low,vlow=roots.vlow;
        float high=roots.interval<roots.count ? roots.stationary[roots.interval] : 1.;
        float vhigh=implicit_value(cell,high==1. ? end : start+high*direction);
        // A strict bracket and zero high node are mutually exclusive. Advance
        // before yielding so a rejected root resumes the remaining SAME cell.
        roots.low=high;roots.vlow=vhigh;roots.interval++;
        if(vlow!=0. && vhigh!=0. && (vlow<0.)!=(vhigh<0.)) {
            float a=low,b=high,va=vlow;
            for(uint iteration=0u;iteration<REG_IMPLICIT_ROOT_BISECTIONS;iteration++) {
                float mid=a+(b-a)*.5;
                if(mid==a || mid==b) break;
                float value=implicit_value(cell,start+mid*direction);
                if(value==0.) {a=mid;b=mid;break;}
                if((value<0.)==(va<0.)) {a=mid;va=value;} else b=mid;
            }
            fraction=b;return true;
        }
        if(vhigh==0.) {fraction=high;return true;}
    }
    return false;
}

// Algebraic primitive: grid surface admission separately requires a boundary
// of the negative region. Keep this first-root helper for numerical consumers.
ImplicitCellHit implicit_cell_hit(ImplicitCell cell,float3 start,float3 end) {
    var roots=implicit_cell_roots(cell,start,end);
    ImplicitCellHit hit={false,0.,float3(0.)};
    hit.found=implicit_cell_next(cell,start,end,roots,hit.fraction);
    if(hit.found) {
        let p=hit.fraction==0. ? start : (hit.fraction==1. ? end : start+hit.fraction*(end-start));
        hit.gradient=implicit_gradient(cell,p);
    }
    return hit;
}
