// Internal dense-grid traversal. Consumers must retain the published snapshot
// for the whole exposure and reject nonfinite fields before shading it.
import implicit_cell;
import vork_constants;

interface IImplicitGrid {
    uint3 dimensions();
    float value(uint3 vertex);
    uint cell_step(uint3 cell);
};
// Endpoint-aware interpolation retains structural zero faces without a tolerance.
float implicit_boundary_mix(float a,float b,float t) {
    return t==0. ? a : (t==1. ? b : lerp(a,b,t));
}

// In an inward orthant F(p+s*u) has seven nonconstant multi-affine
// coefficients. Its inclusion-minimal nonzero monomials determine the local
// sign: positive minimal terms dominate all higher divisible terms near p.
// A negative minimal term is isolated on its coordinate subface. Free axes
// can choose either sign, so any minimal term containing one proves support.
bool implicit_negative_term(float coefficient,uint support,uint free_axes,uint reverse_axes) {
    bool oriented=(countbits(support&reverse_axes)&1u)!=0u ? coefficient>0. : coefficient<0.;
    return (uint((support&free_axes)!=0u) | uint(oriented))!=0u;
}
bool implicit_cell_negative_near(ImplicitCell cell,float3 p) {
    float x=implicit_boundary_mix(
        implicit_boundary_mix(cell.values[1]-cell.values[0],cell.values[3]-cell.values[2],p.y),
        implicit_boundary_mix(cell.values[5]-cell.values[4],cell.values[7]-cell.values[6],p.y),p.z);
    float y=implicit_boundary_mix(
        implicit_boundary_mix(cell.values[2]-cell.values[0],cell.values[3]-cell.values[1],p.x),
        implicit_boundary_mix(cell.values[6]-cell.values[4],cell.values[7]-cell.values[5],p.x),p.z);
    float z=implicit_boundary_mix(
        implicit_boundary_mix(cell.values[4]-cell.values[0],cell.values[5]-cell.values[1],p.x),
        implicit_boundary_mix(cell.values[6]-cell.values[2],cell.values[7]-cell.values[3],p.x),p.y);
    float xy0=(cell.values[3]-cell.values[2])-(cell.values[1]-cell.values[0]);
    float xy1=(cell.values[7]-cell.values[6])-(cell.values[5]-cell.values[4]);
    float xy=implicit_boundary_mix(xy0,xy1,p.z);
    float xz=implicit_boundary_mix(
        (cell.values[5]-cell.values[4])-(cell.values[1]-cell.values[0]),
        (cell.values[7]-cell.values[6])-(cell.values[3]-cell.values[2]),p.y);
    float yz=implicit_boundary_mix(
        (cell.values[6]-cell.values[4])-(cell.values[2]-cell.values[0]),
        (cell.values[7]-cell.values[5])-(cell.values[3]-cell.values[1]),p.x);
    float xyz=xy1-xy0;
    uint free_axes=0u,reverse_axes=0u;
    for(uint axis=0u;axis<3u;axis++) {
        if(p[axis]>0. && p[axis]<1.) free_axes|=1u<<axis;
        else if(p[axis]==1.) reverse_axes|=1u<<axis;
    }
    // Encode the seven supports as bits. Nonzero singleton coefficients
    // suppress every larger support containing that axis; nonzero pairs
    // suppress the triple. This is the complete three-axis subset lattice.
    uint present=uint(x!=0.) | (uint(y!=0.)<<1u) | (uint(xy!=0.)<<2u)
        | (uint(z!=0.)<<3u) | (uint(xz!=0.)<<4u) | (uint(yz!=0.)<<5u) | (uint(xyz!=0.)<<6u);
    uint suppressed=(x!=0. ? 84u : 0u) | (y!=0. ? 100u : 0u) | (z!=0. ? 112u : 0u)
        | ((present&52u)!=0u ? 64u : 0u);
    uint negative=uint(implicit_negative_term(x,1u,free_axes,reverse_axes))
        | (uint(implicit_negative_term(y,2u,free_axes,reverse_axes))<<1u)
        | (uint(implicit_negative_term(xy,3u,free_axes,reverse_axes))<<2u)
        | (uint(implicit_negative_term(z,4u,free_axes,reverse_axes))<<3u)
        | (uint(implicit_negative_term(xz,5u,free_axes,reverse_axes))<<4u)
        | (uint(implicit_negative_term(yz,6u,free_axes,reverse_axes))<<5u)
        | (uint(implicit_negative_term(xyz,7u,free_axes,reverse_axes))<<6u);
    return (present&~suppressed&negative)!=0u;
}

// Qualify a zero at a sample-cell face/edge/vertex against ALL incident
// neighborhoods, not just the DDA-owned cell or a distant negative corner.
// The candidate is already an algebraic root; retain its existing numerical
// conditioning rather than adding a scalar or coordinate epsilon here.
bool implicit_negative_boundary<G:IImplicitGrid>(G grid,float threshold,int3 cell,float3 p,bool cell_negative,out bool valid) {
    valid=true;
    if(all(p>float3(0.)) && all(p<float3(1.))) return cell_negative;
    int3 lower=cell;uint3 alternatives=uint3(1u);
    for(uint axis=0u;axis<3u;axis++) if(p[axis]==0. || p[axis]==1.) {
        if(p[axis]==0.) lower[axis]--;
        alternatives[axis]=2u;
    }
    let dims=grid.dimensions();bool admitted=false;
    for(uint z=0u;z<alternatives.z;z++) for(uint y=0u;y<alternatives.y;y++) for(uint x=0u;x<alternatives.x;x++) {
        let incident=lower+int3(x,y,z);
        if(any(incident<int3(-1)) || any(incident>=int3(dims))) continue;
        ImplicitCell values;float magnitude=abs(threshold),minimum=REG_FP32_MAX;
        for(uint corner=0u;corner<8u;corner++) {
            float f=grid.value(uint3(clamp(incident+int3(corner&1u,(corner>>1u)&1u,corner>>2u),int3(0),int3(dims)-1)));
            if(!isfinite(f)) {valid=false;return false;}
            values.values[corner]=f;magnitude=max(magnitude,abs(f));minimum=min(minimum,f);
        }
        // Continue loading after admission: all incident samples must be finite,
        // exactly as with the original preloaded union, including other cells.
        if(admitted || minimum>=threshold || magnitude==0.) continue;
        for(uint corner=0u;corner<8u;corner++) values.values[corner]=values.values[corner]/magnitude-threshold/magnitude;
        let local=p+float3(cell-incident);
        if(implicit_value(values,local)<0. || implicit_cell_negative_near(values,local)) admitted=true;
    }
    return admitted;
}

struct ImplicitGridHit {
    bool found;
    bool valid;
    bool boundary;
    float distance;
    float3 gradient;
    uint3 cell;
};

// Sign of the clamped half-cell extension, scaled before interpolation.
float implicit_boundary_value<G:IImplicitGrid>(G grid,float threshold,float3 p) {
 uint3 dims=grid.dimensions();float3 q=clamp((p+1.)*.5*float3(dims)-.5,0.,float3(dims-1u));
 uint3 c=min(uint3(floor(q)),dims-2u);float3 t=q-float3(c);
 float f[8];float magnitude=abs(threshold);
 for(uint k=0;k<8;k++) {f[k]=grid.value(c+uint3(k&1u,(k>>1u)&1u,k>>2u));magnitude=max(magnitude,abs(f[k]));}
 float sum=0.;
 if(magnitude>0.) for(uint k=0;k<8;k++) {
  float3 w=lerp(1.-t,t,float3(k&1u,(k>>1u)&1u,k>>2u));sum+=(f[k]/magnitude-threshold/magnitude)*w.x*w.y*w.z;
 }
 return sum;
}

// Grid samples are cell centres in local [-1,1]^3. Direction is NOT normalized: retaining
// the caller's parameterization preserves world ray distances under affine TRS.
ImplicitGridHit implicit_grid_hit<G:IImplicitGrid>(G grid,float threshold,
    float3 origin,float3 direction,float minimum,float maximum) {
    ImplicitGridHit hit={false,true,false,maximum,float3(0.),uint3(0u)};
    uint3 dims=grid.dimensions();
    if(any(dims<uint3(2u)) || !isfinite(threshold) || !all(isfinite(origin))
        || !all(isfinite(direction)) || !isfinite(minimum) || !isfinite(maximum)
        || all(direction==float3(0.))) {
        hit.valid=false;return hit;
    }
    // Offsetting a continuation or short shadow chord can exhaust its interval.
    // A finite empty segment has no crossing; it is not an invalid field.
    if(minimum>maximum) return hit;
    // Cell-centred data extends constantly through each outer half-cell.
    // Close negative regions at the physical domain faces, without moving nodes.
    float box_near=-REG_FP32_MAX,box_far=REG_FP32_MAX;
    float3 entry_normal=0.,exit_normal=0.;
    [unroll] for(uint axis=0u;axis<3u;axis++) {
        if(direction[axis]==0.) {
            if(origin[axis]<-1. || origin[axis]>1.) return hit;
        } else {
            float a=(-1.-origin[axis])/direction[axis];
            float b=(1.-origin[axis])/direction[axis];
            float first=min(a,b),last=max(a,b);
            if(first>box_near) {box_near=first;entry_normal=0.;entry_normal[axis]=direction[axis]>0. ? -1. : 1.;}
            if(last<box_far) {box_far=last;exit_normal=0.;exit_normal[axis]=direction[axis]>0. ? 1. : -1.;}
        }
    }
    float near=max(minimum,box_near),far=min(maximum,box_far);
    if(near>far) return hit;
    if(near==box_near && implicit_boundary_value(grid,threshold,origin+near*direction)<0.) {
        hit.found=true;hit.boundary=true;hit.distance=near;hit.gradient=entry_normal;return hit;
    }
    float3 scale=float3(dims)*.5;
    float3 position=(origin+near*direction+1.)*scale-.5;
    int3 cell=int3(floor(position));
    [unroll] for(uint axis=0u;axis<3u;axis++) {
        if(direction[axis]<0. && position[axis]==float(cell[axis])) cell[axis]--;
    }
    cell=clamp(cell,int3(-1),int3(dims)-1);
    uint entry_axes=0u;float3 entry_local=0.;
    for(;;) {
        // Recompute boundary parameters from the original ray, avoiding drift
        // from repeatedly adding a rounded cell stride across a large grid.
        let stride=threshold==0. && all(cell>=int3(0)) && all(cell<int3(dims)-1) ? grid.cell_step(uint3(cell)) : 1u;
        int3 vertices=cell;
        float3 boundary=float3(far);
        [unroll] for(uint axis=0u;axis<3u;axis++) {
            if(direction[axis]!=0.) {
                int vertex=stride==1u ? cell[axis]+(direction[axis]>0. ? 1 : 0)
                    : min((cell[axis]/int(stride)+(direction[axis]>0. ? 1 : 0))*int(stride),int(dims[axis])-1);
                vertices[axis]=vertex;
                boundary[axis]=(clamp((float(vertex)+.5)/scale[axis]-1.,-1.,1.)-origin[axis])/direction[axis];
            }
        }
        float end=max(near,min(far,min(boundary.x,min(boundary.y,boundary.z))));
        if(stride==1u) {
        ImplicitCell values;
        float magnitude=abs(threshold);
        float low_value=REG_FP32_MAX,high_value=-REG_FP32_MAX;
        [unroll] for(uint i=0u;i<8u;i++) {
            float v=grid.value(uint3(clamp(cell+int3(i&1u,(i>>1u)&1u,(i>>2u)&1u),int3(0),int3(dims)-1)));
            if(!isfinite(v)) {hit.valid=false;return hit;}
            values.values[i]=v;magnitude=max(magnitude,abs(v));
            low_value=min(low_value,v);high_value=max(high_value,v);
        }
        // Scale before subtracting: two finite opposite extrema must not
        // overflow the threshold subtraction or the polynomial coefficients.
        if(magnitude>0. && low_value<=threshold && high_value>=threshold) {
            [unroll] for(uint i=0u;i<8u;i++) values.values[i]=values.values[i]/magnitude-threshold/magnitude;
            float3 a=clamp((origin+near*direction+1.)*scale-.5-float3(cell),0.,1.);
            float3 b=clamp((origin+end*direction+1.)*scale-.5-float3(cell),0.,1.);
            // DDA crossing identities retain exact local faces without
            // rounding the candidate's global index or snapping near roots.
            for(uint axis=0u;axis<3u;axis++) {
                if((entry_axes&(1u<<axis))!=0u) a[axis]=entry_local[axis];
                if(direction[axis]!=0. && boundary[axis]<=end && vertices[axis]>=0 && vertices[axis]<int(dims[axis]))
                    b[axis]=float(vertices[axis]-cell[axis]);
            }
            var roots=implicit_cell_roots(values,a,b);float fraction;
            while(implicit_cell_next(values,a,b,roots,fraction)) {
                let local=fraction==0. ? a : (fraction==1. ? b : a+fraction*(b-a));
                bool valid;
                let admitted=implicit_negative_boundary(grid,threshold,cell,local,low_value<threshold,valid);
                if(!valid) {hit.valid=false;return hit;}
                if(!admitted) continue;
                hit.found=true;hit.distance=near+(end-near)*fraction;
                // Magnitude is irrelevant to the normal; retaining the scaled
                // derivative avoids overflowing large, valid authored fields.
                hit.gradient=implicit_gradient(values,local)*scale;hit.cell=uint3(max(cell,int3(0)));
                return hit;
            }
        }
        }
        if(end>=far) {
            if(far==box_far && implicit_boundary_value(grid,threshold,origin+far*direction)<0.) {
                hit.found=true;hit.boundary=true;hit.distance=far;hit.gradient=exit_normal;
            }
            return hit;
        }
        // Advance every tied axis, including zero-length visits on cell faces.
        // Each iteration advances a cell: there is no authored or silent cap.
        if(stride>1u) {
            let p=(origin+end*direction+1.)*scale-.5;
            var next=int3(floor(p));
            [unroll] for(uint axis=0u;axis<3u;axis++) {
                if(direction[axis]<0. && p[axis]==float(next[axis])) next[axis]--;
                // Reprojection can round behind an already crossed cell face.
                // Never undo progress on another axis while skipping a block.
                cell[axis]=direction[axis]<0. ? min(cell[axis],next[axis]) : max(cell[axis],next[axis]);
            }
        }
        entry_axes=0u;
        [unroll] for(uint axis=0u;axis<3u;axis++) {
            if(direction[axis]!=0. && boundary[axis]<=end) {
                cell[axis]=vertices[axis]-(direction[axis]<0. ? 1 : 0);
                if(vertices[axis]>=0 && vertices[axis]<int(dims[axis])) {
                    entry_axes|=1u<<axis;
                    entry_local[axis]=direction[axis]<0. ? 1. : 0.;
                }
            }
        }
        if(any(cell<int3(-1)) || any(cell>=int3(dims))) return hit;
        near=end;
    }
}
