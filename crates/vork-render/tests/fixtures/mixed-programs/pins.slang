import vork;
struct Inputs { Geometry paddle; };
Closure surface(ShadingContext ctx, Inputs inputs) {
    Closure c={}; c.base_color=float3(.55,.60,.66); c.base_metalness=1.; c.specular_roughness=.24; return c;
}
void compute(ComputeContext ctx, Inputs inputs) {
    uint i=ctx.id.x;
    if (i>=144u) return;
    float x=(float(i%12u)-5.5)*.018;
    float z=(float(i/12u)-5.5)*.018;
    if(ctx.dt==0.) { positions_store(i,float3(x,.052,z)); velocities_store(i,0.); return; }
    float y=positions_load(i).y;
    float v=velocities_load(i);
    // Unit mass oscillator. Semi-implicit Euler, 240 Hz; no timed pin waveform.
    v+=(-240.*(y-.052)-12.*v)*ctx.dt;
    y+=v*ctx.dt;
    float ceiling=1e4;
    // Vertical point-vs-native-triangle test: lowest intersecting paddle face.
    for(uint t=0;t<geometry_triangle_count(inputs.paddle);++t) {
        uint3 ids=geometry_triangle_load(inputs.paddle,t);
        float3 a=geometry_to_world(inputs.paddle,geometry_POSITION_load(inputs.paddle,ids.x));
        float3 b=geometry_to_world(inputs.paddle,geometry_POSITION_load(inputs.paddle,ids.y));
        float3 c=geometry_to_world(inputs.paddle,geometry_POSITION_load(inputs.paddle,ids.z));
        float2 ab=b.xz-a.xz, ac=c.xz-a.xz, q=float2(x,z)-a.xz;
        float d=ab.x*ac.y-ab.y*ac.x;
        if(abs(d)<1e-9) continue;
        float u=(q.x*ac.y-q.y*ac.x)/d;
        float w=(ab.x*q.y-ab.y*q.x)/d;
        if(u>=0. && w>=0. && u+w<=1.) ceiling=min(ceiling,a.y+u*(b.y-a.y)+w*(c.y-a.y));
    }
    // Pin head top is centre+33.5 mm. Inelastic unilateral contact.
    if(y+.0335>ceiling) { y=ceiling-.0335; v=min(v,0.); }
    y=max(y,.018);
    positions_store(i,float3(x,y,z)); velocities_store(i,v);
}
