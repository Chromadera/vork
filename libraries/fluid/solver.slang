import fluid.particles;

// APIC/FLIP solver. Include after the authored configuration and boundary functions.
// The coarse Zhu–Bridson boundary drives pressure; Yu–Turk density drives rendering.
// Neighbour passes use separate generations or independent pressure colours.
// Length is in solver units; time is seconds.
uint index(int3 p) {p=clamp(p,int3(0),int3(phi_dims)-1);return uint(p.x)+phi_dims.x*(uint(p.y)+phi_dims.y*uint(p.z));}
float3 point(int3 p) {return (float3(p)+.5)*(2.*DOMAIN)/float(phi_dims.x)+DOMAIN_ORIGIN-DOMAIN_EXTENT;}
float3 collide(float3 p) {
    for(uint iteration=0;iteration<3;iteration++) {
        float f=container(p)+CONTACT_OFFSET;if(f<=0.) break;
        float3 g=float3(container(p+float3(.0001,0,0))-container(p-float3(.0001,0,0)),
                        container(p+float3(0,.0001,0))-container(p-float3(0,.0001,0)),
                        container(p+float3(0,0,.0001))-container(p-float3(0,0,.0001)));
        if(dot(g,g)>0.) p-=normalize(g)*f;
    }
    return p;
}
float spline(float x) {x=abs(x);return x<.5 ? .75-x*x : (x<1.5 ? .5*(1.5-x)*(1.5-x) : 0.);}
float3 sample_grid(float3 p,bool previous,out float3x3 affine) {
    float3 result=0.;affine=float3x3(0.);float dx=(2.*DOMAIN)/float(phi_dims.x);
    for(uint axis=0;axis<3;axis++) {
        float3 q=(p-DOMAIN_ORIGIN+DOMAIN_EXTENT)/dx-.5;q[axis]+=.5;
        int3 a=int3(floor(q-.5));float3 row=0.;
        for(int z=0;z<3;z++) for(int y=0;y<3;y++) for(int x=0;x<3;x++) {
            int3 c=a+int3(x,y,z);float3 d=float3(c)-q;
            float w=spline(d.x)*spline(d.y)*spline(d.z);
            float v=(previous ? transfer_load(index(c)) : vel_load(index(c)))[axis];result[axis]+=w*v;row+=w*v*d*(4./dx);
        }
        affine[axis]=row;
    }
    return result;
}
float3 sample_velocity(float3 p,out float3x3 affine) {return sample_grid(p,false,affine);}
float3 velocity(float3 p) {float3x3 affine;return sample_velocity(p,affine);}
float pressure_at(int3 p) {return pa_load(index(p));}
// Marker reconstruction uses the existing typed compute buffers. The sorted
// linked lists make each bin independent of dispatch order. One link per marker
// avoids reserving particle slots in empty space; domain escapes reject publication.
// Exact compact ellipsoid gathers. The enclosing sphere rejects only zero weights.
float marker_field(float3 p) {
 // Private raw reconstruction: +infinity certifies solid samples. The public
 // field is finite again after marker_publish; status reports failures.
 if(container(p)>=0.) return asfloat(0x7f800000u);
 float support=asfloat(~bounds_load(6u));int3 lo,hi;query_bins(p,support,lo,hi);
 lo=max(lo,int3(bounds_load(0),bounds_load(1),bounds_load(2)));
 hi=min(hi,int3(~bounds_load(3),~bounds_load(4),~bounds_load(5)));
 float density=0.;
 for(int z=lo.z;z<=hi.z;z++) for(int y=lo.y;y<=hi.y;y++) for(int x=lo.x;x<=hi.x;x++) {
  for(uint key=bins_load(bin_index(int3(x,y,z)));key!=0xffffffffu;key=links_load(key)) {
   if(landed_load(key)>0u) continue;
   float4 centre=centres_load(key);float3 relative=p-centre.xyz;
   if(dot(relative,relative)>centre.w*centre.w) continue;
   float3x3 metric=float3x3(metric_x_load(key),metric_y_load(key),metric_z_load(key));
   density+=amplitude_load(key)*particle_cubic(length(mul(metric,relative)));
  }
 }
 // The empty compact-support plateau is air, at every correction offset.
 return density>0. ? KERNEL_SUPPORT*(.6-density) : 2.*DOMAIN;
}
float pressure_field(float3 p) {
    float radius=MARKER_SPACING*(.045/.013);
    int3 lo=max(int3(floor((p-radius-DOMAIN_ORIGIN+DOMAIN_EXTENT)*(.5*float(BIN_RES)/DOMAIN))),int3(0));
    int3 hi=min(int3(floor((p+radius-DOMAIN_ORIGIN+DOMAIN_EXTENT)*(.5*float(BIN_RES)/DOMAIN))),int3(BIN_DIMS)-1);
    float weight=0.;float3 centroid=0.;
    for(int z=lo.z;z<=hi.z;z++) for(int y=lo.y;y<=hi.y;y++) for(int x=lo.x;x<=hi.x;x++) {
        for(uint key=bins_load(bin_index(int3(x,y,z)));key!=0xffffffffu;key=links_load(key)) {
            if(landed_load(key)>0u) continue;
            float3 delta=markers_load(key)-p;
            float q=saturate(1.-dot(delta,delta)/(radius*radius));
            float w=q*q*q;weight+=w;centroid+=w*delta;
        }
    }
    // Zhu–Bridson weighted particle centres define the pressure boundary.
    // Display anisotropy never changes this pressure occupancy.
    float f=weight>1e-8 ? length(centroid/weight)-.85*MARKER_SPACING : radius;
    return max(f,container(p));
}
// APIC transfer on a MAC grid (Jiang et al. 2015). Quadratic B-splines
// have D = dx^2/4 I, so the fitted affine rows need no matrix inversion.
float particle_velocity(float3 p,uint axis,out float mass) {
    float dx=(2.*DOMAIN)/float(phi_dims.x),support=1.5*dx;
    int3 lo=max(int3(floor((p-support-DOMAIN_ORIGIN+DOMAIN_EXTENT)*(.5*float(BIN_RES)/DOMAIN))),int3(0));
    int3 hi=min(int3(floor((p+support-DOMAIN_ORIGIN+DOMAIN_EXTENT)*(.5*float(BIN_RES)/DOMAIN))),int3(BIN_DIMS)-1);
    float weight=0.,sum=0.;
    for(int z=lo.z;z<=hi.z;z++) for(int y=lo.y;y<=hi.y;y++) for(int x=lo.x;x<=hi.x;x++) {
        for(uint key=bins_load(bin_index(int3(x,y,z)));key!=0xffffffffu;key=links_load(key)) {
            if(landed_load(key)>0u) continue;
            float3 delta=p-markers_load(key),d=delta/dx;
            float v=spline(d.x)*spline(d.y)*spline(d.z);
            float3 row=axis==0u ? affine_x_load(key) : (axis==1u ? affine_y_load(key) : affine_z_load(key));
            weight+=v;sum+=v*(marker_velocity_load(key)[axis]+(1.-FLIP_BLEND)*dot(row,delta));
        }
    }
    mass=weight;return weight>0. ? sum/weight : 0.;
}
#include "fluid/volume.slang"

float rest_density(float3 p) {
 float dx=2.*DOMAIN/float(phi_dims.x);int3 lo,hi;query_bins(p,1.5*dx,lo,hi);float mass=0.;
 for(int z=lo.z;z<=hi.z;z++) for(int y=lo.y;y<=hi.y;y++) for(int x=lo.x;x<=hi.x;x++)
 for(uint key=bins_load(bin_index(int3(x,y,z)));key!=0xffffffffu;key=links_load(key)) {
  if(landed_load(key)>0u) continue;float3 d=(p-markers_load(key))/dx;
  mass+=spline(d.x)*spline(d.y)*spline(d.z);
 }
 return mass*pow(MARKER_SPACING/dx,3.);
}
float fluid_capacity(float3 p) {
 float dx=2.*DOMAIN/float(phi_dims.x);if(container(p)<-2.6*dx) return 1.;float sum=0.;
 for(int z=0;z<6;z++) for(int y=0;y<6;y++) for(int x=0;x<6;x++) {
  float3 d=(float3(x,y,z)+.5)*.5-1.5;
  float weight=.125*spline(d.x)*spline(d.y)*spline(d.z);
  sum+=weight*saturate(.5-container(p+dx*d)/(.5*dx));
 }
 return sum;
}

// Reuse unpublished display storage during the pressure solve. The builder
// supplies a separate range only when the author selected a smaller display.
float density_boundary_value(uint id) {
 if(surface_count>=2u*phi_count) return surface_load(id);
 return density_boundary_load(id);
}
void density_boundary_set(uint id,float value) {
 if(surface_count>=2u*phi_count) surface_store(id,value);
 else density_boundary_store(id,value);
}


void compute(ComputeContext ctx,Inputs inputs) {
    uint id=ctx.id.x;int stage=inputs.stage;
#ifdef VORK_FLUID_STATUS_BOUND
    if(stage==0 && id==0) fluid_status_store(0,0u);
#endif
    // Status is sticky and checked at the output fence. Ordinary simulation
    // kernels need no receipt read; failing producers retain the first cause.
    // Certify a bulk neighbourhood before allowing signed density feedback.
    // Two pressure cells cover the quadratic B-spline support.
    if(stage==31 || stage==32 || stage==33) {
     if(id>=phi_count) return;
     if(stage==31) {density_boundary_set(id,phi_load(id)>=0. ? 1. : 0.);return;}
     uint source=stage==32 ? 0u : phi_count,dest=stage==32 ? phi_count : 0u;
     float boundary=density_boundary_value(source+id);
     if(boundary<.5) {
      int3 c=int3(id%phi_dims.x,(id/phi_dims.x)%phi_dims.y,id/(phi_dims.x*phi_dims.y));
      for(int z=-1;z<=1;z++) for(int y=-1;y<=1;y++) for(int x=-1;x<=1;x++)
       boundary=max(boundary,density_boundary_value(source+index(c+int3(x,y,z))));
     }
     density_boundary_set(dest+id,boundary);return;
    }


    if(stage==30) {
     uint first=id*256u;if(first>=markers_count) return;uint count=0u;
     for(uint j=first;j<min(first+256u,markers_count);j++) count+=landed_load(j)!=3u ? 1u : 0u;
     mass_blocks_store(id,count);return;
    }

    // Conservative unresolved floor film: each marker deposits its rest volume
    // once, using a bilinear partition of unity. No repeat coverage stamping.
    if(stage==26 || stage==27) {
        uint count=stage==26 ? wetness_count : film_partial_count;
        uint first=id*256u;if(first>=count) return;
        float sum=0.;
        for(uint j=first;j<min(first+256u,count);j++)
            sum+=stage==26 ? wetness_load(j) : film_partial_load(j);
        if(stage==26) film_partial_store(id,sum);else film_total_store(id,sum);
        return;
    }
    if(stage==24 || stage==25) {
        if(id>=wetness_count) return;
        if(stage==25) {wetness_store(id,wet_next_load(id));return;}
        int2 c=int2(id%wetness_dims.x,id/wetness_dims.x);
        float dx=2.*DOMAIN/float(wetness_dims.x);
        float3 p=float3((float(c.x)+.5)*dx+DOMAIN_ORIGIN.x-DOMAIN_EXTENT.x,0.,(float(c.y)+.5)*dx+DOMAIN_ORIGIN.z-DOMAIN_EXTENT.z);p.y=film_height(p.xz);
        float wet=wetness_load(id),laplace=0.;
        for(int axis=0;axis<2;axis++) for(int sign=-1;sign<=1;sign+=2) {
            int2 q=c;q[axis]+=sign;q=clamp(q,int2(0),int2(wetness_dims.x,wetness_dims.z)-1);
            // Zero normal flux at the basin footprint and domain boundary.
            float2 centre=(float2(q)+.5)*dx+DOMAIN_ORIGIN.xz-DOMAIN_EXTENT.xz;
            if(film_accept(centre) && film_accept(p.xz))
                laplace+=wetness_load(uint(q.x)+wetness_dims.x*uint(q.y))-wet;
        }
        // Effective spreading of a wettable finish, 5e-5 m²/s. This is a
        // conservative depth diffusion closure, not resolved contact-line dynamics.
        float diffusivity=FILM_DIFFUSIVITY/(METRES_PER_UNIT*METRES_PER_UNIT);
        wet=max(0.,wet+min(.24,diffusivity*ctx.dt/(dx*dx))*laplace);
        int3 lo,hi;query_bins(p,2.*dx,lo,hi);float deposit=0.;
        for(int z=lo.z;z<=hi.z;z++) for(int y=lo.y;y<=hi.y;y++) for(int x=lo.x;x<=hi.x;x++)
        for(uint key=bins_load(bin_index(int3(x,y,z)));key!=0xffffffffu;key=links_load(key)) {
            if(landed_load(key)!=1u) continue;
            float2 marker=markers_load(key).xz;
            float2 d=abs(marker-p.xz)/dx;
            float weight=max(0.,1.-d.x)*max(0.,1.-d.y);
            if(weight<=0. || !film_accept(p.xz)) continue;
            // Renormalize the four-cell partition at the ceramic footprint.
            // Water touching the floor beside the wall must not remain a bead,
            // and its film volume must not disappear beneath the basin.
            int2 first=int2(floor((marker-DOMAIN_ORIGIN.xz+DOMAIN_EXTENT.xz)/dx-.5));float total=0.;
            for(int z=0;z<2;z++) for(int x=0;x<2;x++) {
                int2 cell=first+int2(x,z);
                if(any(cell<0) || any(cell>=int2(wetness_dims.x,wetness_dims.z))) continue;
                float2 centre=(float2(cell)+.5)*dx+DOMAIN_ORIGIN.xz-DOMAIN_EXTENT.xz;
                if(!film_accept(centre)) continue;
                float2 distance=abs(marker-centre)/dx;
                total+=max(0.,1.-distance.x)*max(0.,1.-distance.y);
            }
            if(total>0.) deposit+=weight/total;
        }
        wet_next_store(id,wet+deposit*MARKER_SPACING*MARKER_SPACING*MARKER_SPACING/(dx*dx));return;
    }
    if(stage==14 || stage==15) {
        if(id>=markers_count) return;
        float3 q;
        if(stage==14) {
            landed_store(id,0u);
            bool active=initial_marker(id,q);
            if(active) q=collide(q); else {q=DOMAIN_ORIGIN;landed_store(id,3u);}
            marker_velocity_store(id,active ? velocity(q) : float3(0.));
            affine_x_store(id,float3(0.));affine_y_store(id,float3(0.));affine_z_store(id,float3(0.));
        } else {
            q=markers_load(id);
            if(landed_load(id)==3u) return;
            if(landed_load(id)>0u) {landed_store(id,2u);return;}
            float3x3 affine;float3 v=sample_velocity(q,affine);
            float3x3 old_affine;
            float3 flip=marker_velocity_load(id)+v-sample_grid(q,true,old_affine);
            marker_velocity_store(id,lerp(v,flip,FLIP_BLEND));
            affine_x_store(id,affine[0]);affine_y_store(id,affine[1]);affine_z_store(id,affine[2]);
            q+=ctx.dt*velocity(q+.5*ctx.dt*v);
            q=collide(q);
            if(q.y<=film_height(q.xz)+CONTACT_OFFSET*1.02 && film_accept(q.xz)) {
                landed_store(id,1u);marker_velocity_store(id,float3(0.));
                affine_x_store(id,float3(0.));affine_y_store(id,float3(0.));affine_z_store(id,float3(0.));
            }
        }
        markers_store(id,q);return;
    }
    if(stage==16) {if(id<7u) bounds_store(id,0xffffffffu);if(id<bins_count) bins_store(id,0xffffffffu);if(id<links_count) links_store(id,0xffffffffu);if(id==0) overflow_store(0,0xffffffffu);return;}
    if(stage==17) {
        if(id>=markers_count) return;
        float3 q=markers_load(id);if(landed_load(id)==3u) return;
        if(!all(isfinite(q)) || any(abs(q-DOMAIN_ORIGIN)>DOMAIN_EXTENT-MARKER_SPACING)) {
            overflow_atomic_min(0,0u);
#ifdef VORK_FLUID_STATUS_BOUND
            fluid_status_atomic_compare_exchange(0,0u,1u);
#endif
            return;
        }
        int3 c=clamp(int3(floor((q-DOMAIN_ORIGIN+DOMAIN_EXTENT)*(.5*float(BIN_RES)/DOMAIN))),int3(0),int3(BIN_DIMS)-1);
        // Insert in ascending marker order. Each atomic minimum retains the
        // smaller ID and carries the displaced larger ID to its successor.
        // Links always increase, so concurrent insertions cannot form cycles.
        uint key=id,old=bins_atomic_min(bin_index(c),key);
        while(old!=0xffffffffu) {
            uint predecessor=min(old,key);key=max(old,key);
            old=links_atomic_min(predecessor,key);
        }
        return;
    }
    if(stage==19) {
        uint3 dims=surface_dims;
        if(id>=dims.x*dims.y*dims.z) return;
        int3 c=int3(id%dims.x,(id/dims.x)%dims.y,id/(dims.x*dims.y));
        float3 q=(float3(c)+.5)*(2.*DOMAIN)/float(dims.x)+DOMAIN_ORIGIN-DOMAIN_EXTENT;
        float f=marker_field(q);
        if(any(c==int3(0)) || any(c==int3(dims)-1)) f=max(f,(2.*DOMAIN)/float(dims.x));
#ifdef VORK_FLUID_STATUS_BOUND
        if(!isfinite(container(q)) || (!isfinite(f) && asuint(f)!=0x7f800000u)) fluid_status_atomic_compare_exchange(0,0u,4u);
#else
        if(overflow_load(0)!=0xffffffffu) f=asfloat(0x7fc00000u);
#endif
        surface_store(id,f);
        return;
    }
    if(stage==11) {
        if(id==0) {
            correction_store(0,0.);correction_store(1,asfloat(0xff800000u));correction_store(2,asfloat(0x7f800000u));
            correction_store(3,0.);correction_store(4,0.);correction_store(5,0.);
#ifdef VORK_FLUID_STATUS_BOUND
            correction_store(6,0.);
#endif
        }
        return;
    }
    // Deterministic two-level reduction. No floating atomics or racing sums;
    // every source value contributes once, in a fixed order at each level.
    if(stage==22 || stage==23) {
        uint count=stage==22 ? volume_count : volume_partial_count;
        uint first=id*256u;if(first>=count) return;
        float3 sum=0.;
        for(uint j=first;j<min(first+256u,count);j++)
            sum+=stage==22 ? volume_load(j) : volume_partial_load(j);
        if(stage==22) volume_partial_store(id,sum);else volume_total_store(id,sum);
        return;
    }
    if(stage==10) {
        if(id!=0 || correction_load(3)!=0.) return;
        float3 sum=0.;
        for(uint j=0;j<volume_total_count;j++) sum+=volume_total_load(j);
        float film=0.;for(uint j=0;j<film_total_count;j++) film+=film_total_load(j);
        float film_dx=2.*DOMAIN/float(wetness_dims.x),surface_dx=2.*DOMAIN/float(surface_dims.x);
        float measured=sum.y+film*film_dx*film_dx/(surface_dx*surface_dx*surface_dx);
        uint particles=0u;for(uint j=0;j<mass_blocks_count;j++) particles+=mass_blocks_load(j);
        float target=float(particles)*pow(MARKER_SPACING/surface_dx,3.);
        float residual=measured-target,offset=correction_load(0);
        float iteration=correction_load(5)+1.;correction_store(5,iteration);
        correction_store(4,residual);
        // Monotone continuous occupancy of the actual published trilinear field.
        // A fixed relative ceiling cannot grow with the liquid's surface area.
#ifdef VORK_FLUID_STATUS_BOUND
        if(abs(residual)<=max(target*0.0001,0.000001)) {
            // The same evaluated field must also be a valid trilinear surface.
            if(sum.x<0.) fluid_status_atomic_compare_exchange(0,0u,5u);
            correction_store(3,1.);return;
        }
        if(!isfinite(residual)) {correction_store(3,-1.);fluid_status_atomic_compare_exchange(0,0u,4u);return;}
        if(correction_load(6)==0. && iteration==1. && residual<0. &&
           sum.x+film*film_dx*film_dx/(surface_dx*surface_dx*surface_dx)<target-max(target*0.0001,0.000001)) {
            // An accepted or over-target evaluation already proves capacity.
            // Otherwise whole negative cells give a conservative lower bound;
            // only inconclusive bounds request the exact-capacity pass.
            correction_store(6,-1.);
        }
        // The first distance-family evaluation is its largest admitted volume.
        // Refuse immediately if the rest ledger lies outside that capacity.
        if(correction_load(6)==1. && iteration==1. && residual<0.) {
            correction_store(3,-1.);fluid_status_atomic_compare_exchange(0,0u,2u);return;
        }
#else
        // Preserve the 0.4.0 correction arithmetic and failure publication for
        // scenes whose six-word correction buffer predates capacity receipts.
        if(abs(residual)<=max(target*0.0001,0.000001)) {correction_store(3,1.);return;}
#endif
        float lo=correction_load(1),hi=correction_load(2);
        if(residual>0.) lo=offset;else hi=offset;
        correction_store(1,lo);correction_store(2,hi);
        if(iteration>=40. || !(lo<hi)) {correction_store(3,-1.);return;}
        // Establish a bracket by expansion when the derivative vanishes;
        // offsets are not limited by the domain's length or a density cap.
        float fallback=isfinite(lo) && isfinite(hi) ? .5*(lo+hi) :
            offset+(isfinite(lo) ? 1. : -1.)*max(2.*DOMAIN,abs(offset));
        float next=iteration<=4. && sum.z>0. ? offset+residual/sum.z : fallback;
        if(!isfinite(next) || next<=lo || next>=hi) next=fallback;
        correction_store(0,next);return;
    }
    if(stage==9) {
        if(id>=volume_count || correction_load(3)!=0.) return;
        uint x=id%surface_dims.x,z=id/surface_dims.x;
        float3 sum=0.;
        if(x+1u<surface_dims.x && z+1u<surface_dims.z) {
            float dx=2.*DOMAIN/float(surface_dims.x),offset=correction_load(0);
            for(uint y=0;y+1u<surface_dims.y;y++) {
                float f[8],g[8];float minimum=1e30,maximum=-1e30;
#ifdef VORK_FLUID_STATUS_BOUND
                bool full_capacity=true;
#endif
                for(uint k=0;k<8;k++) {
                    int3 c=int3(x+(k&1u),y+((k>>1u)&1u),z+(k>>2u));
                    uint at=uint(c.x)+surface_dims.x*(uint(c.y)+surface_dims.y*uint(c.z));
                    float3 q=(float3(c)+.5)*dx+DOMAIN_ORIGIN-DOMAIN_EXTENT;
                    float source=surface_load(at),wall=container(q);
                    f[k]=published_field(source,wall,offset,c);
                    if(any(c==int3(0)) || any(c==int3(surface_dims)-1)) wall=max(wall,dx);
#ifdef VORK_FLUID_STATUS_BOUND
                    full_capacity=full_capacity && wall<0. && !(isfinite(source) && source>=2.*DOMAIN);
#endif
                    g[k]=isfinite(source) && source<2.*DOMAIN && source+offset>wall ? 1. : 0.;
                    minimum=min(minimum,f[k]);maximum=max(maximum,f[k]);
                }
#ifdef VORK_FLUID_STATUS_BOUND
                sum.x+=full_capacity ? 1. : 0.;
                // Negative sentinel survives the fixed reductions. It is used
                // only on an accepted offset, not during root iterations.
                if(minimum==0. && maximum==0.) sum.x=-1e30;
#endif
                if(minimum>=0.) continue;
                if(maximum<=0.) {sum.y+=1.;continue;}
                float volume=occupied_cell(f);
                // A finite-difference derivative proposes steps only; acceptance
                // always uses the independently evaluated physical residual.
                float delta=.001*dx;float shifted[8];
                for(uint k=0;k<8;k++) shifted[k]=f[k]+delta*g[k];
                sum.y+=volume;sum.z+=max(0.,(volume-occupied_cell(shifted))/delta);
            }
        }
        volume_store(id,sum);return;
    }
    if(stage==20) {
        if(id<surface_count) {
            int3 c=int3(id%surface_dims.x,(id/surface_dims.x)%surface_dims.y,id/(surface_dims.x*surface_dims.y));
            float3 q=(float3(c)+.5)*(2.*DOMAIN)/float(surface_dims.x)+DOMAIN_ORIGIN-DOMAIN_EXTENT;
            float source=surface_load(id);
            float f=source>=2.*DOMAIN && isfinite(source) ? source : published_field(source,container(q),correction_load(0),c);
#ifdef VORK_FLUID_STATUS_BOUND
            if(correction_load(3)!=1.) {fluid_status_atomic_compare_exchange(0,0u,3u);return;}
            if(!isfinite(f)) {fluid_status_atomic_compare_exchange(0,0u,4u);return;}
            surface_store(id,f);
#else
            surface_store(id,correction_load(3)==1. ? f : asfloat(0x7fc00000u));
#endif
        }
        return;
    }
    if(stage==21) {
        if(id>=phi_count) return;
        int3 c=int3(id%phi_dims.x,(id/phi_dims.x)%phi_dims.y,id/(phi_dims.x*phi_dims.y));
        float f=pressure_field(point(c));
        if(any(c==int3(0)) || any(c==int3(phi_dims)-1)) f=max(f,(2.*DOMAIN)/float(phi_dims.x));
        phi_store(id,f);return;
    }
    if(id>=phi_count) return;
    int3 cell=int3(id%phi_dims.x,(id/phi_dims.x)%phi_dims.y,id/(phi_dims.x*phi_dims.y));
    float3 p=point(cell);float dx=(2.*DOMAIN)/float(phi_dims.x);float dt=ctx.dt;
    if(stage==0) {
        float pool=initial_field(p);
        phi_store(id,pool);
        float3 initial=initial_velocity(p,dx);
        vel_store(id,initial);adv_store(id,initial);float hydro=initial_pressure(p);pa_store(id,hydro);div_store(id,0.);return;
    }
    if(stage==1) {
        float3 v=0.,before=0.;
        for(int axis=0;axis<3;axis++) {
            int3 d=int3(0);d[axis]=1;float3 face=p;face[axis]-=.5*dx;
            float mass;v[axis]=particle_velocity(face,uint(axis),mass);
            before[axis]=v[axis];
            v[axis]=mass>0. ? v[axis]-(axis==1 ? GRAVITY*dt : 0.) : 0.;
            if(container(p)>0. || container(point(cell-d))>0.) v[axis]=0.;
        }
        if(FLIP_BLEND>0.) transfer_store(id,before);adv_store(id,boundary_velocity(p,ctx.time,v));return;
    }
    if(stage==2) {
        float divergence=0.;
        for(int a=0;a<3;a++) {int3 d=int3(0);d[a]=1;divergence+=(adv_load(index(cell+d))[a]-adv_load(id)[a])/dx;}
        float expansion=0.;
        if(phi_load(id)<0. && container(p)<0.) {
            float rho=rest_density(p),capacity=fluid_capacity(p),error=rho-capacity;
            if(density_boundary_value(id)>.5) error=max(error,0.);
            // Relax density over a resolved cell's gravity/capillary time.
            // An instantaneous rest-volume projection injects excess motion.
            float acceleration=max(GRAVITY,CAPILLARY/(dx*dx));
            float tau=sqrt(dx/max(acceleration,1e-8));
            expansion=error/max(max(rho,capacity),1e-8)*(1.-exp(-dt/tau))/dt;
        }
        div_store(id,divergence-expansion);
        float pressure=0.;
        if(abs(phi_load(id))<2.*dx && container(p)<-dx) {
            float3 gradient=0.,diagonal=0.;float3x3 hessian=float3x3(0.);
            float center=phi_load(id);
            for(int a=0;a<3;a++) {
                int3 d=int3(0);d[a]=1;
                float lo=phi_load(index(cell-d)),hi=phi_load(index(cell+d));
                gradient[a]=(hi-lo)/(2.*dx);diagonal[a]=(hi-2.*center+lo)/(dx*dx);
                hessian[a][a]=diagonal[a];
                for(int b=0;b<a;b++) {
                    int3 e=int3(0);e[b]=1;
                    float mixed=(phi_load(index(cell+d+e))-phi_load(index(cell+d-e))
                        -phi_load(index(cell-d+e))+phi_load(index(cell-d-e)))/(4.*dx*dx);
                    hessian[a][b]=mixed;hessian[b][a]=mixed;
                }
            }
            float g2=dot(gradient,gradient);
            if(g2>1e-8) {
                float curvature=(g2*(diagonal.x+diagonal.y+diagonal.z)-dot(gradient,mul(hessian,gradient)))/(g2*sqrt(g2));
                // Authored surface tension / density, converted from SI.
                // Laplace pressure uses the same pressure-potential units as g*h.
                pressure=CAPILLARY*clamp(curvature,-2./dx,2./dx);
            }
        }
        capillary_store(id,pressure);return;
    }
    if(stage==3 || stage==4) {
        // Red/black SOR: axial neighbours always belong to the other colour,
        // so each pass safely updates one pressure generation in place.
        if(((cell.x+cell.y+cell.z)&1)!=(stage-3)) return;
        float sum=0.,count=0.;
        if(phi_load(id)<0. && container(p)<0.) {
            for(int a=0;a<3;a++) for(int sign=-1;sign<=1;sign+=2) {
                int3 d=int3(0);d[a]=sign;float3 q=point(cell+d);
                if(container(q)>0.) continue;
                float other=phi_load(index(cell+d));
                if(other<0.) {count+=1.;sum+=pressure_at(cell+d);}
                else {
                    // Ghost-fluid pressure boundary at the actual zero crossing.
                    float theta=max(.01,phi_load(id)/(phi_load(id)-other));
                    count+=1./theta;sum+=capillary_load(id)/theta;
                }
            }
            sum=count>0. ? (sum-div_load(id)*dx*dx/dt)/count : 0.;
        }
        pa_store(id,lerp(pa_load(id),sum,1.7));return;
    }
    if(stage==5) {
        float3 v=adv_load(id);
        for(int axis=0;axis<3;axis++) {
            int3 d=int3(0);d[axis]=1;float3 face=p;face[axis]-=.5*dx;
            float left=phi_load(index(cell-d)),right=phi_load(id);
            float theta=1.;
            if((left<0.) != (right<0.)) theta=max(.01,min(left,right)/(min(left,right)-max(left,right)));
            float pl=left<0. ? pa_load(index(cell-d)) : capillary_load(id);
            float pr=right<0. ? pa_load(id) : capillary_load(index(cell-d));
            if(left<0. || right<0.) v[axis]-=dt*(pr-pl)/(dx*theta);
            if(container(p)>0. || container(point(cell-d))>0.) v[axis]=0.;
        }
        vel_store(id,v);return;
    }
}
