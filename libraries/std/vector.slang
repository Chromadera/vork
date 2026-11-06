// Normal-frame and surface-gradient numerics.
float3 vector_safe_normalize(float3 v) { float n=length(v);return n>0.?v/n:float3(0.); }
// Surface-gradient bump: callers supply screen derivatives (raster), analytical
// derivatives or ray differentials. No invented derivatives in ray/path contexts.
public float3 bump(float3 normal,float3 position_dx,float3 position_dy,float height_dx,float height_dy,
    float strength=1.,float distance=0.001,bool invert=false) {
    float3 rx=cross(position_dy,normal),ry=cross(normal,position_dx);
    float determinant=dot(position_dx,rx);
    if(abs(determinant)<1.e-20) return normal;
    float sign=determinant<0.?-1.:1.;
    float3 gradient=sign*(height_dx*rx+height_dy*ry);
    float3 perturbed=vector_safe_normalize(abs(determinant)*normal-(invert?-distance:distance)*gradient);
    return vector_safe_normalize(lerp(normal,perturbed,saturate(strength)));
}

// Offset is measured in pixels around the shaded point. Evaluate the complete
// height expression at that offset, including its texture-coordinate mapping.
public interface BumpHeight { float evaluate(float2 pixel_offset); }
public float3 bump<T:BumpHeight>(T height, float3 normal, float3 pixel_x, float3 pixel_y,
    float strength=1., float distance=.001, float filter_width=.1, bool invert=false) {
    float width=max(filter_width,0.);
    if(width==0.) return normal;
    float center=height.evaluate(float2(0.));
    return bump(normal,pixel_x*width,pixel_y*width,
        height.evaluate(float2(width,0.))-center,
        height.evaluate(float2(0.,width))-center,strength,distance,invert);
}
