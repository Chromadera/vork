// Clean-room photographic glare kernels; Blender 5.2 socket names/defaults.
// Bloom is authored separately in bloom.slang.
import vork;
public enum GlareType { Streaks, Ghosts, Fog_Glow, Simple_Star };
public enum GlareQuality { High, Medium, Low };
public struct Inputs {
    GlareType type = GlareType::Streaks;
    GlareQuality quality = GlareQuality::Medium;
    [InputRange(0.0, 3.402823466e+38, true, true)] float threshold = 1.;
    [InputRange(0.0, 1.0, true, true)] float smoothness = 0.1;
    bool clamp = false;
    [InputRange(0.0, 3.402823466e+38, true, true)] float maximum = 10.;
    [InputRange(0.0, 3.402823466e+38, true, true)] float strength = 1.;
    [InputRange(0.0, 3.402823466e+38, true, true)] float saturation = 1.;
    [InputRange(0.0, 3.402823466e+38, true, true)] float3 tint = float3(1);
    [InputRange(0.0, 1.0, true, true)] float size = 0.5;
    [InputRange(1.0, 16.0, true, true)] int streaks = 4;
    float streaks_angle = 0.;
    [InputRange(1.0, 5.0, true, true)] int iterations = 3;
    [InputRange(0.0, 1.0, true, true)] float fade = 0.9;
    [InputRange(0.0, 1.0, true, true)] float color_modulation = 0.25;
    bool diagonal = true;
};
static const float GLARE_PI = 3.141592653589793;
static const int GLARE_LINE_TAPS = 128;
import highlight;
float3 glare_spectral(float3 c,float phase,float modulation) {
    float3 weights=0.5+0.5*cos(phase+float3(0.,2.*GLARE_PI/3.,4.*GLARE_PI/3.));
    return c*lerp(float3(1),weights*2.,saturate(modulation));
}
float3 glare_streaks(float2 uv,float2 extent,Inputs inputs,bool star) {
    int rays=star ? 4 : clamp(inputs.streaks,1,16);
    float angle=star ? (inputs.diagonal ? GLARE_PI/4. : 0.) : inputs.streaks_angle;
    float radius=max(1.,min(extent.x,extent.y)*saturate(inputs.size));
    float3 total=float3(0);
    int taps=inputs.quality==GlareQuality::High ? GLARE_LINE_TAPS : (inputs.quality==GlareQuality::Low ? 48 : 96);
    float fade=clamp(inputs.fade,0.,0.999);
    for(int ray=0;ray<rays;ray++) {
        float a=angle+2.*GLARE_PI*float(ray)/float(rays);
        // Blender angles are counterclockwise; image UV grows downward.
        float2 direction=float2(cos(a),-sin(a))/extent;
        float3 line=float3(0);float weight=0.;
        for(int i=1;i<=taps;i++) {
            float t=float(i)/float(taps);
            float distance=t*radius;
            float w=pow(fade,distance/(star ? pow(2.,float(inputs.iterations)-3.) : max(1.,float(inputs.iterations))));
            float jacobian=1.;
            line+=glare_spectral(highlight(uv+direction*distance,inputs.threshold,inputs.smoothness,inputs.clamp,inputs.maximum),t*2.*GLARE_PI,inputs.color_modulation)*w*jacobian;
            weight+=w*jacobian;
        }
        total+=line/max(weight,0.000001);
    }
    return total/float(rays);
}
float3 glare_ghost_sample(float2 uv,float2 extent,Inputs inputs) {
    float3 sum=float3(0);float weight=0.;
    float sigma=max(1.,float(inputs.iterations)*3.);
    int half_taps=inputs.quality==GlareQuality::High ? 12 : (inputs.quality==GlareQuality::Low ? 4 : 8);
    for(int y=-half_taps;y<=half_taps;y++) for(int x=-half_taps;x<=half_taps;x++) {
        float2 q=float2(x,y)*(3./float(half_taps));
        float w=exp(-0.5*dot(q,q));
        sum+=highlight(uv+q*sigma/extent,inputs.threshold,inputs.smoothness,inputs.clamp,inputs.maximum)*w;weight+=w;
    }
    return sum/weight;
}
float3 glare_ghosts(float2 uv,float2 extent,Inputs inputs) {
    float2 p=uv-.5;
    float3 result=float3(0);
    int count=clamp(inputs.iterations,1,5)*2;
    for(int i=0;i<count;i++) {
        float scale=(float(i)+1.)*0.5;
        float direction=(i%2)==0 ? -1. : 1.;
        float2 q=.5+direction*p*scale;
        float vignette=max(0.,1.-length(q-.5)*1.41421356237);
        result+=glare_spectral(glare_ghost_sample(q,extent,inputs),float(i)*2.39996323,inputs.color_modulation)*vignette*vignette;
    }
    return result/float(count);
}
float3 glare_fog_glow(float2 uv,float2 extent,Inputs inputs) {
    float sigma=pow(2.,saturate(inputs.size)*8.-2.);
    int half_taps=inputs.quality==GlareQuality::High ? 12 : (inputs.quality==GlareQuality::Low ? 4 : 8);
    float3 sum=float3(0);float weight=0.;
    for(int y=-half_taps;y<=half_taps;y++) for(int x=-half_taps;x<=half_taps;x++) {
        float2 q=float2(x,y)*(3./float(half_taps));
        float w=exp(-.5*dot(q,q));
        sum+=highlight(uv+q*sigma/extent,inputs.threshold,inputs.smoothness,inputs.clamp,inputs.maximum)*w;weight+=w;
    }
    return sum/weight;
}
public float4 composite(CompositeContext ctx, Inputs inputs) { float2 uv=ctx.uv; float2 extent=ctx.size; float time=ctx.time;
    if(inputs.strength<=0.) return float4(0.);
    float3 effect;
    if(inputs.type==GlareType::Ghosts) effect=glare_ghosts(uv,extent,inputs);
    else if(inputs.type==GlareType::Fog_Glow) effect=glare_fog_glow(uv,extent,inputs);
    else effect=glare_streaks(uv,extent,inputs,inputs.type==GlareType::Simple_Star);
    float peak=max(effect.r,max(effect.g,effect.b));
    effect=lerp(float3(peak),effect,max(inputs.saturation,0.));
    return float4(max(effect,0.)*max(inputs.tint,0.)*max(inputs.strength,0.),0.);
}
