import vork;
public float3 highlight(float2 uv,float threshold,float smoothness,bool clamp_highlight,float maximum) {
    if(any(uv<0.) || any(uv>1.)) return float3(0);
    float3 c=max(compositor_sample(uv).rgb,0.);
    float peak=max(c.r,max(c.g,c.b));
    threshold=max(threshold,0.);
    float width=min(threshold,max(smoothness,0.));
    float highlight=max(peak-threshold,0.);
    if(width>0.) {float d=max(width-abs(peak-threshold),0.);highlight+=d*d/(4.*width);}
    if(clamp_highlight) highlight=min(highlight,max(maximum,0.));
    return peak>0. ? c*(highlight/peak) : float3(0);
}
