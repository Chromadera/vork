// Basalt, highlands and small-scale relief, evaluated on the moving moon.
struct Inputs {};
uint hash(uint x) {x^=x>>16;x*=0x7feb352du;x^=x>>15;x*=0x846ca68bu;return x^(x>>16);}
float random(uint x) {return float(hash(x)&0xffffffu)/16777216.;}
float noise(float3 p) {
    let cell=int3(floor(p));let f=frac(p);let w=f*f*(3.-2.*f);
    var value=0.;
    for(int k=0;k<2;k++) for(int j=0;j<2;j++) for(int i=0;i<2;i++) {
        let c=cell+int3(i,j,k);
        let id=hash(uint(c.x))^hash(uint(c.y)+0x1234567u)^hash(uint(c.z)+0x7654321u);
        let weight=lerp(1.-w.x,w.x,float(i))*lerp(1.-w.y,w.y,float(j))*lerp(1.-w.z,w.z,float(k));
        value+=weight*random(id);
    }
    return value;
}

Closure surface(ShadingContext ctx,Inputs inputs) {
    let p=normalize(world_to_object(ctx,ctx.POSITION));
    let mare=noise(p*3.5)+.32*noise(p*9.);
    let dust=noise(p*65.);
    Closure c=Closure();c.base_color=lerp(float3(.085,.075,.068),float3(.72,.75,.78),smoothstep(.38,.85,mare))*(.75+.5*dust);
    c.specular_roughness=.38;c.specular_weight=.65;
    // Faint blue airglow makes the far hemisphere readable through the lens.
    c.emission_color=c.base_color*float3(.12,.65,1.);c.emission_luminance=2.;
    return c;
}
