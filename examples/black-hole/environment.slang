// Distant source directions only. Lensing belongs to the shared free-flight
// field, so camera, reflected and illuminating stars use the same transport.
struct Inputs {float star_energy=.08;};
uint hash(uint x) {x^=x>>16;x*=0x7feb352du;x^=x>>15;x*=0x846ca68bu;return x^(x>>16);}
float random(uint x) {return float(hash(x)&0xffffffu)/16777216.;}

float3 environment(EnvironmentContext ctx,Inputs inputs) {
    let d=normalize(ctx.direction);let cell=int3(floor(d*128.));
    var value=float3(.000015,.000022,.000045);
    // A dim galactic band supplies large-scale source structure behind the hole.
    let band=exp(-pow((d.y+.23*d.x-.12*d.z)/.055,2.));
    value+=band*float3(.00015,.00013,.00018);
    for(int k=-1;k<=1;k++) for(int j=-1;j<=1;j++) for(int i=-1;i<=1;i++) {
        let c=cell+int3(i,j,k);
        let id=hash(uint(c.x))^hash(uint(c.y)+0x1234567u)^hash(uint(c.z)+0x7654321u);
        if(random(id)>.15) continue;
        let candidate=(float3(c)+float3(random(id+1u),random(id+2u),random(id+3u)))/128.;
        if(abs(length(candidate)-1.)>1./256.) continue;
        let star=normalize(candidate);let distance=length(d-star);let radius=.00055;
        value+=exp(-distance*distance/(2.*radius*radius))*lerp(float3(.5,.65,1.),float3(1.,.65,.3),random(id+999u))*(.8+8.*pow(random(id+117u),6.));
    }
    return value*inputs.star_energy;
}
