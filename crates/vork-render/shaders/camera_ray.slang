import frame;
import vork_sampling;

// The camera ray through `position` (pixels), for perspective and orthographic
// cameras. Its direction is the raster view (camera_view), exact for an
// orthographic camera rather than a per-pixel difference of rounded points.
public void camera_ray(float2 position, uint2 size, float2 lens, out float3 origin, out float3 direction) {
    let ndc=position/float2(size)*float2(2.,-2.)+float2(-1.,1.);
    camera_ray_line(ndc,frame,origin,direction);
    if (frame.post.w>0.) {
        let forward=-float3(frame.view[0].z,frame.view[1].z,frame.view[2].z);
        let focus=frame.eye.xyz+direction*(frame.lens.y/dot(direction,forward));
        let right=float3(frame.view[0].x,frame.view[1].x,frame.view[2].x);
        let up=float3(frame.view[0].y,frame.view[1].y,frame.view[2].y);
        let projection_y=frame.projection.x;
        let radius=frame.post.w*frame.lens.y*frame.lens.w/projection_y;
        origin=frame.eye.xyz+radius*(right*lens.x+up*lens.y);
        direction=normalize(focus-origin);
    }
}
// A pass's clip offset moves geometry by (+x, -y) pixels, so a sample at
// lattice offset `offset` lies at (-x, +y) from the pixel centre in the nominal camera.
public float2 sample_position(uint2 pixel, float2 offset) {return float2(pixel)+0.5+float2(-1.,1.)*offset;}
// Barycentrics (of corners 1 and 2) where the ray meets the world triangle
// (Möller–Trumbore), clamped into the triangle against rounding at its edges.
// `a` is the area-scaled cosine between the ray and the triangle's normal: only
// a degenerate or edge-on triangle takes its centre.
public float2 triangle_barycentrics(float3 origin, float3 direction, float3 p0, float3 p1, float3 p2) {
    let e1=p1-p0;let e2=p2-p0;let q=cross(direction,e2);let a=dot(e1,q);
    if (!(abs(a)>1e-6*length(cross(e1,e2)))) {return float2(1./3.);}
    let s=(origin-p0)/a;let r=cross(s,e1);
    var b=max(float2(dot(s,q),dot(direction,r)),float2(0.));
    if (b.x+b.y>1.) {b/=b.x+b.y;}
    return b;
}
public float2 camera_sample_lens(StructuredBuffer<uint4> samples, uint sample) {
    if (frame.post.w<=0.) {return float2(0.);}
    return aperture_point(asfloat(samples[sample*2u+3u].xy),frame.aperture);
}

// The camera's clipped transport segment starts at its projected near plane.
public float camera_near_distance(float3 origin,float3 direction) {
    let forward=-float3(frame.view[0].z,frame.view[1].z,frame.view[2].z);
    return max(0.,(frame.params.x-dot(origin-frame.eye.xyz,forward))/dot(direction,forward));
}

public float camera_far_distance(float3 origin,float3 direction) {
    let forward=-float3(frame.view[0].z,frame.view[1].z,frame.view[2].z);
    return max(0.,(frame.params.y-dot(origin-frame.eye.xyz,forward))/dot(direction,forward));
}
