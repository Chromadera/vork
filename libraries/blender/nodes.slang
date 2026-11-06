// Blender socket helpers independently expressed over vork's public context.
// Callers pass evaluated source socket values; Blender owns authoring defaults.
import vork;
import std.openpbr_core;

// Diffuse BSDF: pinned EEVEE uses Lambert and ignores Roughness.
public Closure Diffuse_BSDF(float3 Color, float Roughness, float3 Normal) {
    Closure c={};c.base_color=Color;c.specular_weight=0.;c.geometry_normal=Normal;
    return c;
}
// Glass BSDF: EEVEE uses its GGX reflection/refraction response.
public Closure Glass_BSDF(float3 Color,float Roughness,float IOR,float3 Normal,
                          float Thin_Film_Thickness,float Thin_Film_IOR) {
    Closure c={};c.base_weight=0.;c.specular_color=Color;
    c.specular_roughness=Roughness;c.specular_ior=IOR;
    c.transmission_weight=1.;c.transmission_color=Color;c.geometry_normal=Normal;
    c.thin_film_weight=Thin_Film_Thickness>0. ? 1. : 0.;
    c.thin_film_thickness=Thin_Film_Thickness*.001;c.thin_film_ior=Thin_Film_IOR;
    return c;
}
public Closure Transparent_BSDF(float3 Color) {
    Closure c={};c.base_weight=0.;c.specular_weight=0.;c.geometry_opacity=0.;
    return c;
}
// Blender's Fresnel node uses geometric front/back orientation for the IOR ratio.
public float Fresnel(float IOR,float3 Normal,ShadingContext ctx) {
    float eta=max(IOR,1.e-5);
    return fresnel(abs(dot(normalize(Normal),normalize(ctx.incoming))),ctx.front_facing?eta:1./eta);
}
public enum MathOperation { ADD, SUBTRACT, MULTIPLY, DIVIDE, MULTIPLY_ADD,
    MINIMUM, MAXIMUM, LESS_THAN, GREATER_THAN, ABSOLUTE, FLOOR, CEIL,
    POWER, FRACTION, TRUNC, SINE, COSINE, TANGENT, ARCTANGENT, SQRT, SIGN,
    RADIANS, DEGREES, EXPONENT, ROUND, MODULO, FLOORED_MODULO };
// Duplicate Blender Value labels receive positional suffixes in legal identifiers.
public float Math(MathOperation Operation,float Value,float Value_001,float Value_002,bool Clamp) {
    float a=Value,b=Value_001,c=Value_002,r=0.;
    switch(Operation) {
        case MathOperation.ADD:r=a+b;break;
        case MathOperation.SUBTRACT:r=a-b;break;
        case MathOperation.MULTIPLY:r=a*b;break;
        case MathOperation.DIVIDE:r=b==0.?0.:a/b;break;
        case MathOperation.MULTIPLY_ADD:r=a*b+c;break;
        case MathOperation.MINIMUM:r=min(a,b);break;
        case MathOperation.MAXIMUM:r=max(a,b);break;
        case MathOperation.LESS_THAN:r=a<b?1.:0.;break;
        case MathOperation.GREATER_THAN:r=a>b?1.:0.;break;
        case MathOperation.ABSOLUTE:r=abs(a);break;
        case MathOperation.FLOOR:r=floor(a);break;
        case MathOperation.CEIL:r=ceil(a);break;
        case MathOperation.POWER:
            if(b==0.) r=1.;
            else if(a==0.) r=0.;
            else if(a>0.) r=pow(a,b);
            else if(b==trunc(b)) r=pow(-a,b)*(frac(abs(b)*.5)==0.?1.:-1.);
            break;
        case MathOperation.FRACTION:r=frac(a);break;
        case MathOperation.TRUNC:r=trunc(a);break;
        case MathOperation.SINE:r=sin(a);break;
        case MathOperation.COSINE:r=cos(a);break;
        case MathOperation.TANGENT:r=tan(a);break;
        case MathOperation.ARCTANGENT:r=atan(a);break;
        case MathOperation.SQRT:r=a>0.?sqrt(a):0.;break;
        case MathOperation.SIGN:r=sign(a);break;
        case MathOperation.RADIANS:r=a*0.017453292519943295;break;
        case MathOperation.DEGREES:r=a*57.29577951308232;break;
        case MathOperation.EXPONENT:r=exp(a);break;
        case MathOperation.ROUND:r=floor(a+.5);break;
        case MathOperation.MODULO:r=b==0.?0.:a-trunc(a/b)*b;break;
        case MathOperation.FLOORED_MODULO:r=b==0.?0.:a-floor(a/b)*b;break;
    }
    return Clamp?saturate(r):r;
}

// Tangent-space Normal Map over the imported active-render-UV tangent frame.
public float3 Normal_Map(float Strength,float3 Color,ShadingContext ctx) {
    float3 n=2.*Color-1.;
    float k=max(Strength,0.);
    float3 mapped=float3(n.xy*k,1.+saturate(k)*(n.z-1.));
    float3 normal=normalize(ctx.NORMAL);
    float3 tangent=normalize(ctx.TANGENT-normal*dot(ctx.TANGENT,normal));
    float3 world=mapped.x*tangent+mapped.y*ctx.tangent_sign*cross(normal,tangent)+mapped.z*normal;
    return dot(world,world)>0.?normalize(world):normal;
}
public float Layer_Weight_Fresnel(float Blend,float3 Normal,ShadingContext ctx) {
    return Fresnel(1./max(1.-Blend,1.e-5),Normal,ctx);
}
