// Internal dense-grid traversal. Consumers must retain the published snapshot
// for the whole exposure and reject nonfinite fields before shading it.
import implicit_cell;
import vork_constants;

interface IImplicitGrid {
    uint3 dimensions();
    float value(uint3 vertex);
    uint cell_step(uint3 cell);
};
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
            let root=implicit_cell_hit(values,a,b);
            if(root.found) {
                hit.found=true;hit.distance=near+(end-near)*root.fraction;
                // Magnitude is irrelevant to the normal; retaining the scaled
                // derivative avoids overflowing large, valid authored fields.
                hit.gradient=root.gradient*scale;hit.cell=uint3(max(cell,int3(0)));
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
        [unroll] for(uint axis=0u;axis<3u;axis++) {
            if(direction[axis]!=0. && boundary[axis]<=end)
                cell[axis]=vertices[axis]-(direction[axis]<0. ? 1 : 0);
        }
        if(any(cell<int3(-1)) || any(cell>=int3(dims))) return hit;
        near=end;
    }
}
