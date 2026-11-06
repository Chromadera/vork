// Blender Mix Color blend modes and Float Curve, independently implemented from
// Blender's manual. No Blender source is incorporated.
import std.color;
public enum BlendMode { Mix, Darken, Multiply, Burn, Lighten, Screen, Dodge, Add,
    Overlay, Soft_Light, Linear_Light, Difference, Exclusion, Subtract, Divide, Hue, Saturation, Color, Value };
public float3 mix(float3 a,float3 b,float factor=0.5,BlendMode mode=BlendMode.Mix,
                  bool clamp_factor=true,bool clamp_result=false) {
    float f=clamp_factor?saturate(factor):factor;
    if(mode==BlendMode.Burn || mode==BlendMode.Dodge) {
        float3 result;
        for(int i=0;i<3;i++) {
            float denominator=mode==BlendMode.Burn?1.-f+f*b[i]:1.-f*b[i];
            result[i]=mode==BlendMode.Burn?(denominator<=0.?0.:1.-(1.-a[i])/denominator):
                (denominator<=0.?1.:a[i]/denominator);
        }
        return saturate(result);
    }
    float3 c=b;
    switch(mode) {
        case BlendMode.Darken: c=min(a,b); break;
        case BlendMode.Multiply: c=a*b; break;
        case BlendMode.Lighten: c=max(a,b); break;
        case BlendMode.Screen: c=1.-(1.-a)*(1.-b); break;
        case BlendMode.Add: c=a+b; break;
        case BlendMode.Overlay:
            for(int i=0;i<3;i++) c[i]=a[i]<0.5?2.*a[i]*b[i]:1.-2.*(1.-a[i])*(1.-b[i]); break;
        case BlendMode.Soft_Light: c=(1.-2.*b)*a*a+2.*b*a; break;
        case BlendMode.Linear_Light: c=a+2.*b-1.; break;
        case BlendMode.Difference: c=abs(a-b); break;
        case BlendMode.Exclusion: c=a+b-2.*a*b; break;
        case BlendMode.Subtract: c=a-b; break;
        case BlendMode.Divide: for(int i=0;i<3;i++) c[i]=b[i]==0.?a[i]:a[i]/b[i]; break;
        case BlendMode.Hue: case BlendMode.Saturation: case BlendMode.Color: case BlendMode.Value: {
            float3 ah=color_to_hsv(a),bh=color_to_hsv(b);
            if(mode==BlendMode.Hue && bh.y!=0.) ah.x=bh.x;
            if(mode==BlendMode.Saturation) ah.y=bh.y;
            if(mode==BlendMode.Color) { ah.x=bh.x; ah.y=bh.y; }
            if(mode==BlendMode.Value) ah.z=bh.z;
            c=color_from_hsv(ah); break;
        }
        default: break;
    }
    c=lerp(a,c,f);
    return clamp_result?saturate(c):c;
}
// A curve is a sorted sequence of knots with explicit cubic Bezier handles.
// Handles permit authored automatic, aligned and vector handle modes without
// making a shader solve authoring constraints for every shaded sample.
public struct CurvePoint { public float2 position; public float2 left; public float2 right; };
float2 curve_bezier(float2 a,float2 b,float2 c,float2 d,float t) {
    float u=1.-t;return u*u*u*a+3.*u*u*t*b+3.*u*t*t*c+t*t*t*d;
}
public float float_curve<let N:int>(float value,CurvePoint points[N],float factor=1.,bool extrapolate=true) {
    if(N==0) return value;
    if(N==1) return lerp(value,points[0].position.y,factor);
    float result=0.;
    if(value<points[0].position.x || value>points[N-1].position.x) {
        int end=value<points[0].position.x?0:N-1;
        float2 handle=end==0?points[end].left:points[end].right;
        float2 delta=handle-points[end].position;
        result=points[end].position.y;
        if(extrapolate && delta.x!=0.) result+=(value-points[end].position.x)*delta.y/delta.x;
    } else {
        int i=0;for(int j=1;j<N-1;j++) if(value>=points[j].position.x) i=j;
        float2 a=points[i].position,d=points[i+1].position;
        float2 b=points[i].right,c=points[i+1].left;
        // Monotone X handles; bisection avoids unstable cubic roots.
        b.x=clamp(b.x,a.x,d.x);c.x=clamp(c.x,a.x,d.x);
        float lo=0.,hi=1.;
        for(int iteration=0;iteration<24;iteration++) {
            float t=(lo+hi)*0.5;
            if(curve_bezier(a,b,c,d,t).x<value) lo=t;else hi=t;
        }
        result=curve_bezier(a,b,c,d,(lo+hi)*0.5).y;
    }
    return lerp(value,result,factor);
}
