// Blender Wave, Brick and Magic textures in scene-linear space. Independent
// implementations of the node definitions documented in Blender's manual; no
// Blender implementation source is incorporated.
import std.procedurals;
static const float BLENDER_TEX_PI = 3.14159265358979323846;
public enum WaveType { Bands, Rings };
public enum WaveDirection { X, Y, Z, Diagonal };
public enum WaveProfile { Sin, Saw, Tri };
public float wave(float3 vector, float scale=5., float distortion=0., float detail=2.,
    float detail_scale=1., float detail_roughness=0.5, float phase_offset=0.,
    WaveType type=WaveType.Bands, WaveDirection direction=WaveDirection.X,
    WaveProfile profile=WaveProfile.Sin) {
    float3 p=vector*scale; float n;
    if(type==WaveType.Bands) n=(direction==WaveDirection.X?p.x:direction==WaveDirection.Y?p.y:direction==WaveDirection.Z?p.z:p.x+p.y+p.z)*20.;
    else n=length(direction==WaveDirection.X?p.yz:direction==WaveDirection.Y?p.xz:p.xy)*20.;
    if(type==WaveType.Rings && direction==WaveDirection.Diagonal) n=length(p)*20.;
    n+=phase_offset;
    if(distortion!=0.) n+=distortion*(noise(p*detail_scale,1.,detail,detail_roughness)*2.-1.);
    if(profile==WaveProfile.Saw) return frac(n/(2.*BLENDER_TEX_PI));
    if(profile==WaveProfile.Tri) return abs(frac(n/(2.*BLENDER_TEX_PI)+0.5)*2.-1.);
    return 0.5+0.5*sin(n-BLENDER_TEX_PI*0.5);
}
public struct BrickResult { public float3 color; public float factor; };
public BrickResult brick(float3 vector, float3 color1=float3(0.8), float3 color2=float3(0.2),
    float3 mortar=float3(0.), float scale=5., float mortar_size=0.02, float mortar_smooth=0.,
    float bias=0., float brick_width=0.5, float row_height=0.25,
    float offset=0.5, int offset_frequency=2, float squash=1., int squash_frequency=2) {
    float2 p=vector.xy*scale; float height=max(row_height,0.000001);int row=int(floor(p.y/height));
    float width=max(brick_width,0.000001);
    if(squash_frequency>0 && row%squash_frequency==0) width*=max(squash,0.000001);
    if(offset_frequency>0 && row%offset_frequency==0) p.x+=width*offset;
    int column=int(floor(p.x/width));float2 local=float2(p.x-float(column)*width,p.y-float(row)*height);
    float dist=min(min(local.x,width-local.x),min(local.y,height-local.y));
    float factor=dist<mortar_size?1.:0.;
    if(mortar_smooth>0. && mortar_size>0.) factor=1.-smoothstep(mortar_size*(1.-mortar_smooth),mortar_size,dist);
    BrickResult result;result.factor=factor;
    result.color=lerp(lerp(color1,color2,saturate(tex_random(int3(column,row,0))+bias)),mortar,factor);return result;
}
public struct MagicResult { public float3 color; public float factor; };
public MagicResult magic(float3 vector,float scale=5.,float distortion=1.,int depth=2) {
    // Coupled trigonometric iteration; each pass feeds the next channel.
    float3 p=vector*scale*5.;
    float3 v=float3(sin(p.x+p.y+p.z),cos(-p.x+p.y-p.z),-cos(-p.x-p.y+p.z));
    for(int i=0;i<clamp(depth,0,10);i++) {
        int axis=i%3;
        v[axis]=sin((v[(axis+1)%3]+v[(axis+2)%3])*distortion);
    }
    MagicResult result;result.color=0.5-0.5*v;result.factor=(result.color.x+result.color.y+result.color.z)/3.;return result;
}
