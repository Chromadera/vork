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

// Return the first root on a closed segment wholly inside the cell. A constant
// zero cell has no isolated surface. No distance-field/Lipschitz assumption is
// made. Bisection evaluates the interpolant, avoiding power-basis cancellation.
ImplicitCellHit implicit_cell_hit(ImplicitCell cell,float3 start,float3 end) {
    ImplicitCellHit hit={false,0.,float3(0.)};
    float3 direction=end-start;
    float4 c=implicit_polynomial(cell,start,direction);
    if(all(c==float4(0.))) return hit;
    float2 stationary;
    uint count=implicit_stationary(c,stationary);
    float low=0.;
    float vlow=implicit_value(cell,start);
    for(uint interval=0u;interval<=count;interval++) {
        float high=interval<count ? stationary[interval] : 1.;
        float vhigh=implicit_value(cell,start+high*direction);
        if(vlow==0. || vhigh==0. || (vlow<0.)!=(vhigh<0.)) {
            if(vlow==0.) high=low;
            else if(vhigh!=0.) {
                for(uint iteration=0u;iteration<REG_IMPLICIT_ROOT_BISECTIONS;iteration++) {
                    float mid=low+(high-low)*.5;
                    if(mid==low || mid==high) break;
                    float value=implicit_value(cell,start+mid*direction);
                    if(value==0.) {low=mid;high=mid;break;}
                    if((value<0.)==(vlow<0.)) {low=mid;vlow=value;} else high=mid;
                }
            }
            hit.found=true;
            hit.fraction=high;
            hit.gradient=implicit_gradient(cell,start+high*direction);
            return hit;
        }
        low=high;vlow=vhigh;
    }
    return hit;
}
