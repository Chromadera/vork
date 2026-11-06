// Camera-aligned local coefficient estimates. View-depth knots parameterize
// the cache; reconstruction integrates in metres on the sampled receiver ray.
import vork_constants;
import vork_sampling;
import vork_prelude;
import vork_medium_types;
import vork_medium_point;
internal extern Frame medium_frame();
// The visibility sample's camera shift accompanies its jittered grid, so both
// lighting and reconstruction use the same ray as retained receiver geometry.
public Frame froxel_sample_frame(Frame f,uint4 sample,float2 disk) {
    if(f.sampling.w>0u) {
        f.temporal.x=f.temporal.x*float(f.sampling.w)+float(sample.x);f.temporal.y*=float(f.sampling.w);
        var shift=float2(asfloat(sample.y),asfloat(sample.z));
        if(f.post.w>0.) {
            let lens=aperture_point(disk,f.aperture);
            let offset=lens*(f.post.w*f.lens.y*f.lens.w/f.projection.x);
            let right=float3(f.view[0].x,f.view[1].x,f.view[2].x);
            let up=float3(f.view[0].y,f.view[1].y,f.view[2].y);
            let displacement=right*offset.x+up*offset.y;
            f.eye.xyz+=displacement;f.view[3].xy-=offset;
            f.vp[3].xy-=offset*f.projection.yx;
            // Translate the lens and shift projection so its rays meet the
            // nominal rays at the focus plane, as sampled visibility does.
            for(uint row=0u;row<4u;row++) {f.inverse_vp[row].xyz+=f.inverse_vp[row].w*displacement;}
            shift+=offset*f.projection.yx/f.lens.y;
        }
        f.inverse_vp[3]-=shift.x*f.inverse_vp[0]+shift.y*f.inverse_vp[1];
        for(uint row=0u;row<4u;row++) {f.vp[row].xy+=f.vp[row].w*shift;}
    }
    return f;
}
#ifndef VORK_MEDIUM_BOUNDARIES
#define VORK_MEDIUM_BOUNDARIES 0
#endif
#if VORK_MEDIUM_BOUNDARIES > 0
internal extern void medium_column_begin(float3 origin);
internal extern void medium_column_end();
internal extern MediumTransport medium_transport_range(float3 o,float3 d,float start,float end,float fraction);
#endif
internal extern uint2 medium_size();
internal extern float4 medium_froxel_read(uint index);
internal extern void medium_froxel_write(uint index,float4 value);
internal extern float2 medium_receiver_support(uint2 pixel);
internal extern MediumTransport medium_transport_unbounded(float3 o,float3 d,float scale,float fraction);
internal extern MediumTransport medium_transport_segment(float3 o,float3 d,MediumEndpoint endpoint,float fraction,uint inside_object);
#ifndef VORK_VOLUME_TILE
#define VORK_VOLUME_TILE 8
#define VORK_VOLUME_SLICES 64
#define VORK_VOLUME_DISTRIBUTION 0.8
#endif
// Ceiling division without overflow for any tile size (the extent is at least one pixel).
uint2 froxel_size() {return (medium_size()-uint2(1u))/uint2(VORK_VOLUME_TILE)+uint2(1u);}
// Shift the sampling lattice so lighting and interpolation describe the same
// cell centres. Exposure samples integrate subcell coverage.
float2 froxel_jitter(Frame f) {
    return (sample_2d(uint2(uint(f.scene.y),0u),uint(f.temporal.x),uint(f.temporal.y),
        REG_SAMPLING_STREAM_VOLUME)-.5)*float(VORK_VOLUME_TILE);
}
float froxel_depth(float t,float2 range) {
    let n=range.x;let f=range.y;
    if (t<=0.) {return 0.;}
    return lerp(n+(f-n)*t,n*pow(f/max(n,REG_POSITIVE_EPSILON),t),VORK_VOLUME_DISTRIBUTION)/f;
}
// Find the actual prefix interval, not an approximate inverse of the depth map.
// Integer bracketing terminates in ceil(log2(slices)) iterations and compares
// the same floating-point knot positions used by lighting and integration.
uint froxel_slice(float fraction,float2 range) {
    var lo=0u;var hi=uint(VORK_VOLUME_SLICES);
    while(lo+1u<hi) {
        let mid=lo+(hi-lo)/2u;
        if(froxel_depth(float(mid)/float(VORK_VOLUME_SLICES),range)<=fraction) {lo=mid;}else{hi=mid;}
    }
    return lo;
}
uint froxel_index(uint2 xy,uint z) {let size=froxel_size();return z*size.x*size.y+xy.y*size.x+xy.x;}
uint froxel_transmission_offset() {let size=froxel_size();return size.x*size.y*(VORK_VOLUME_SLICES+2u);}
void froxel_store(uint2 xy,uint z,MediumTransport m) {
    let index=froxel_index(xy,z);
    medium_froxel_write(index,float4(m.source,0.));
    medium_froxel_write(index+froxel_transmission_offset(),float4(m.transmission,0.));
}
MediumTransport froxel_fetch(uint2 xy,uint z) {
    let index=froxel_index(xy,z);
    return MediumTransport(medium_froxel_read(index).rgb,medium_froxel_read(index+froxel_transmission_offset()).rgb,float3(0.));
}
// Local extinction and source density divided by max(extinction, 1/m).
// This is an invertible numeric encoding, not a density bound. Opaque cells
// retain finite equilibrium radiance without reconstructing an enormous source.
public struct FroxelCell {public float3 extinction;public float3 source;};
public float3 froxel_source_response(float3 sigma,float distance,float3 transmission) {
    let finite=medium_integrated_coefficient(max(sigma,float3(1.)),sigma,distance,transmission);
    // An infinite optical-depth record is an opaque cell. Its retained source
    // is already its equilibrium radiance; no reciprocal infinity is needed.
    return select(isinf(sigma),float3(1.),finite);
}
public FroxelCell froxel_estimate(MediumTransport m,float distance) {
    if(distance==0.) {return FroxelCell(float3(0.),float3(0.));}
    let sigma=medium_ratio(m.optical_depth,float3(distance));
    let response=froxel_source_response(sigma,distance,m.transmission);
    return FroxelCell(sigma,select(m.source==float3(0.),float3(0.),medium_ratio(m.source,response)));
}
public MediumTransport froxel_integrate(FroxelCell cell,float distance) {
    if(distance==0.) {return MediumTransport(float3(0.),float3(1.),float3(0.));}
    let t=exp(-cell.extinction*distance);
    let response=froxel_source_response(cell.extinction,distance,t);
    return MediumTransport(cell.source*response,t,cell.extinction*distance);
}
// Interpolate the original coefficients in a common scale, keeping the
// intermediate source density normalized throughout the convex combination.
public struct FroxelBlend {public float3 scale;public float3 extinction;public float3 source;};
public void froxel_blend(inout FroxelBlend blend,FroxelCell cell,float weight) {
    if(weight<=0.) {return;}
    let scale=max(max(blend.scale,cell.extinction),float3(1.));
    // In the unit scale, the encoded coefficients are already the original
    // coefficients. Avoid reciprocal/rescaling work without changing them.
    if(all(scale==float3(1.))) {
        blend.extinction+=weight*cell.extinction;blend.source+=weight*cell.source;
        blend.scale=float3(1.);return;
    }
    let opaque=isinf(scale);
    let previous=select(opaque,select(isinf(blend.scale),float3(1.),float3(0.)),medium_ratio(blend.scale,scale));
    let rate=select(opaque,select(isinf(cell.extinction),float3(1.),float3(0.)),medium_ratio(cell.extinction,scale));
    let normalization=select(opaque,select(isinf(cell.extinction),float3(1.),float3(0.)),medium_ratio(max(cell.extinction,float3(1.)),scale));
    blend.extinction=blend.extinction*previous+weight*rate;
    blend.source=blend.source*previous+(weight*cell.source)*normalization;
    blend.scale=scale;
}
public FroxelCell froxel_blended(FroxelBlend blend) {
    if(all(blend.scale==float3(1.)) && all(blend.extinction<=float3(1.))) {return FroxelCell(blend.extinction,blend.source);}
    let sigma=blend.extinction*blend.scale;
    return FroxelCell(sigma,medium_ratio(blend.source,max(blend.extinction,medium_ratio(float3(1.),blend.scale))));
}
// Plane zero is reserved for the current sample's receiver support. A column
// contributes through bilinear reconstruction within one tile of its jittered
// centre. Include an extra pixel on both sides for integer/edge rounding.
public void froxel_receiver_support(uint3 cell) {
    let size=froxel_size();if(any(cell.xy>=size)) {return;}
    let extent=medium_size();let frame=medium_frame();
    let centre=clamp(float2(cell.xy*VORK_VOLUME_TILE+VORK_VOLUME_TILE/2u)+froxel_jitter(frame),float2(-.5),float2(extent)-.5);
    let lo=int2(clamp(floor(centre-float(VORK_VOLUME_TILE)-1.),float2(0.),float2(extent-1u)));
    let hi=int2(clamp(ceil(centre+float(VORK_VOLUME_TILE)+1.),float2(0.),float2(extent-1u)));
    var support=float2(0.);
    for(int y=lo.y;y<=hi.y;y++) {for(int x=lo.x;x<=hi.x;x++) {
        support=max(support,medium_receiver_support(uint2(x,y)));
    }}
    medium_froxel_write(froxel_index(cell.xy,0u),float4(support,0.,0.));
}
void froxel_light_cell(uint3 cell) {
    let xy=cell.xy;let z=cell.z;let size=froxel_size();
    if(any(xy>=size) || z>VORK_VOLUME_SLICES) {return;}
    let frame=medium_frame();let range=frame.params.xy;let frame_index=uint(frame.scene.y);
    let pixel=clamp(float2(xy*VORK_VOLUME_TILE+VORK_VOLUME_TILE/2u)+froxel_jitter(frame),float2(-.5),float2(medium_size())-.5);
    let ndc=(pixel+.5)/float2(medium_size())*float2(2.,-2.)+float2(-1.,1.);
    float3 origin;float3 direction;camera_ray_line(ndc,frame,origin,direction);
    let forward=-float3(frame.view[0].z,frame.view[1].z,frame.view[2].z);
    let limit=range.y/dot(direction,forward);let samples=max(uint(frame.temporal.y),1u);
    let stratum=(float(uint(frame.temporal.x)%samples)+.5)/float(samples);
    let start=limit*froxel_depth(float(z)/float(VORK_VOLUME_SLICES),range);
    let end=limit*froxel_depth(float(min(z+1u,VORK_VOLUME_SLICES))/float(VORK_VOLUME_SLICES),range);
    let support=medium_froxel_read(froxel_index(xy,0u)).xy;
    let seed=pixel_hash((xy.x+xy.y*size.x)^pixel_hash(z+frame_index*VORK_VOLUME_SLICES));
    let fraction=frac(stratum+float(seed)*REG_RADICAL_INVERSE_SCALE);
    if(z==VORK_VOLUME_SLICES) {
#if VORK_MEDIUM_BOUNDARIES > 0
        medium_column_end();
#endif
        if(support.y==0.) {
            froxel_store(xy,z+1u,MediumTransport(float3(0.),float3(1.),float3(0.)));
            return;
        }
        froxel_store(xy,z+1u,medium_transport_unbounded(origin+direction*limit,direction,max(limit,1.),fraction));
        return;
    }
    // Only completed cells are reconstructed from the cache. Every terminal
    // interval is integrated on the actual receiver ray, so a cell ending past
    // every supported finite receiver cannot contribute. Background requires the
    // complete grid, independently of its finite-depth support or knot rounding.
    if(support.y==0. && froxel_depth(float(z+1u)/float(VORK_VOLUME_SLICES),range)>support.x) {
        froxel_store(xy,z+1u,MediumTransport(float3(0.),float3(0.),float3(0.)));
        return;
    }
#if VORK_MEDIUM_BOUNDARIES > 0
    let transport=medium_transport_range(origin,direction,start,end,fraction);
#else
    let transport=medium_transport_segment(origin+direction*start,direction,medium_receiver(end-start),fraction,0xffffffffu);
#endif
    let estimate=froxel_estimate(transport,end-start);
    froxel_store(xy,z+1u,MediumTransport(estimate.source,estimate.extinction,float3(0.)));
}
// Bounded columns retain ordered intervals across a short depth batch so one
// invocation does not serialize the entire column's light connections. World-only grids
// keep independent cell lighting, where there are no boundary queries to reuse.
public void froxel_light(uint3 cell) {
#if VORK_MEDIUM_BOUNDARIES > 0
    let size=froxel_size();
    if(any(cell.xy>=size)) {return;}
    let frame=medium_frame();
    let pixel=clamp(float2(cell.xy*VORK_VOLUME_TILE+VORK_VOLUME_TILE/2u)+froxel_jitter(frame),float2(-.5),float2(medium_size())-.5);
    let ndc=(pixel+.5)/float2(medium_size())*float2(2.,-2.)+float2(-1.,1.);
    float3 origin;float3 direction;camera_ray_line(ndc,frame,origin,direction);
    medium_column_begin(origin);
    let start=cell.z*REG_MEDIUM_LIGHTING_DEPTH_BATCH;
    let end=min(start+REG_MEDIUM_LIGHTING_DEPTH_BATCH,uint(VORK_VOLUME_SLICES)+1u);
    for(uint z=start;z<end;z++) {froxel_light_cell(uint3(cell.xy,z));}
#else
    froxel_light_cell(cell);
#endif
}
// Imported by composition; exporting also retains it in unrelated native entries.
public MediumTransport froxel_transport(int2 pixel,float fraction,bool background=false) {
    fraction=clamp(fraction,0.,1.);
    let camera=medium_frame();let range=camera.params.xy;
    let z=froxel_slice(fraction,range);
    let z0=froxel_depth(float(z)/float(VORK_VOLUME_SLICES),range);

    let ndc=(float2(pixel)+.5)/float2(medium_size())*float2(2.,-2.)+float2(-1.,1.);
    float3 origin;float3 direction;camera_ray_line(ndc,camera,origin,direction);
    let forward=-float3(camera.view[0].z,camera.view[1].z,camera.view[2].z);
    let cosine=dot(direction,forward);let limit=range.y/cosine;
    let xy=(float2(pixel)-float(VORK_VOLUME_TILE/2u)-froxel_jitter(camera))/float(VORK_VOLUME_TILE);
    let base=floor(xy);let blend=frac(xy);let size=froxel_size();
    var source=float3(0.);var transmission=float3(1.);
    // Bilinear filtering acts on local coefficients at each view-depth cell.
    // Prefixes are then derived in order with ds = d(view depth)/cosine for
    // this receiver. Different cells retain their own source/extinction ratio.
    let count=background ? uint(VORK_VOLUME_SLICES) : z;
    for(uint depth=0u;depth<count;depth++) {
        FroxelBlend mixture={};
        for(var y=0u;y<2u;y++) {for(var x=0u;x<2u;x++) {
            let cell=uint2(clamp(base+float2(float(x),float(y)),float2(0.),float2(size-1u)));
            let weight=select(x==1u,blend.x,1.-blend.x)*select(y==1u,blend.y,1.-blend.y);
            if(weight<=0.) {continue;}
            let local=froxel_fetch(cell,depth+1u);
            froxel_blend(mixture,FroxelCell(local.transmission,local.source),weight);
        }}
        let a=froxel_depth(float(depth)/float(VORK_VOLUME_SLICES),range);
        let b=froxel_depth(float(depth+1u)/float(VORK_VOLUME_SLICES),range);
        let m=froxel_integrate(froxel_blended(mixture),limit*(b-a));
        source+=medium_received_source(transmission,m.source);transmission*=m.transmission;
        if(!any(transmission>float3(0.))) {return MediumTransport(source,transmission,float3(0.));}
    }
    if(background) {
        var tail=MediumTransport(float3(0.),float3(0.),float3(0.));
        for(var y=0u;y<2u;y++) {for(var x=0u;x<2u;x++) {
            let cell=uint2(clamp(base+float2(float(x),float(y)),float2(0.),float2(size-1u)));
            let weight=select(x==1u,blend.x,1.-blend.x)*select(y==1u,blend.y,1.-blend.y);
            let m=froxel_fetch(cell,VORK_VOLUME_SLICES+1u);
            tail.source+=weight*m.source;tail.transmission+=weight*m.transmission;
        }}
        source+=medium_received_source(transmission,tail.source);transmission*=tail.transmission;
    }
    if(!background && fraction>z0) {
        // The complete terminal cell may contain light behind the receiver.
        // Evaluate only its visible interval on the actual sampled camera ray.
        let samples=max(uint(camera.temporal.y),1u);
        let seed=pixel_hash(uint(pixel.x)+uint(pixel.y)*medium_size().x+pixel_hash(z+uint(camera.scene.y)*VORK_VOLUME_SLICES));
        let sample=frac((float(uint(camera.temporal.x)%samples)+.5)/float(samples)+float(seed)*REG_RADICAL_INVERSE_SCALE);
        // No column cache: bounded intervals must be queried on this pixel ray.
        let residual=medium_transport_segment(origin+direction*(limit*z0),direction,medium_receiver(limit*(fraction-z0)),sample,0xffffffffu);
        source+=medium_received_source(transmission,residual.source);
        transmission*=residual.transmission;
    }
    return MediumTransport(source,transmission,float3(0.));
}
