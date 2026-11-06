// Schwarzschild optical metric, isotropic coordinates, G=c=M=1.
// The author chooses the polygonal approximation and its error/cost tradeoff.
struct Inputs {};
float3 gradient(float3 p) {
    let r=max(length(p),.6);let a=.5;let u=a/r;
    let n=pow(1.+u,3.)/(1.-u);
    let g=-3.*a/(r*(r+a))-a/(r*(r-a));
    let t=saturate((r-500.)/500.);let taper=1.-t*t*(3.-2.*t);
    let derivative=-6.*t*(1.-t)/500.;
    return (n*g*taper+(n-1.)*derivative)/(1.+(n-1.)*taper)*p/r;
}
float redshift(float3 p) {
    let r=max(length(p),.6);let u=.5/r;
    let t=saturate((r-500.)/500.);let taper=1.-t*t*(3.-2.*t);
    return 1.+(pow((1.+u)/(1.-u),4.)-1.)*taper;
}
PropagationStep propagation(PropagationContext ctx,Inputs inputs) {
    PropagationStep s;s.absorbed=length(ctx.position)<=.601;
    let r=length(ctx.position);
    s.distance=min(.02*r,max(r-.6,.0001)*.25);
    let g=gradient(ctx.position);
    // Staggered directions live at segment midpoints. Reconstruct the tangent
    // at this endpoint before applying the symmetric kick on the unequal steps.
    var tangent=ctx.direction;
    for(uint j=0u;j<3u;j++) tangent=normalize(ctx.direction+.5*ctx.previous_distance*(g-tangent*dot(g,tangent)));
    s.direction=normalize(ctx.direction+.5*(ctx.previous_distance+s.distance)*(g-tangent*dot(g,tangent)));
    s.radiance=float3(redshift(ctx.position)/redshift(ctx.position+s.direction*s.distance));
    return s;
}
