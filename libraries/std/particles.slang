// Particle surface mathematics: Yu & Turk, ACM TOG 32(1), 2013, equations 7–16.
// Independent implementation from the published formulas; no upstream code copied.
struct ParticleMoments {
    float weight;
    float3 first;
    float3x3 second;
    uint neighbours;
};
void particle_moment(inout ParticleMoments m,float3 relative,float radius) {
    float q=length(relative)/radius;
    if(q>=1.) return;
    float w=1.-q*q*q;
    m.weight+=w;m.first+=w*relative;
    m.second+=w*float3x3(relative.x*relative,relative.y*relative,relative.z*relative);
    m.neighbours++;
}
float3x3 particle_covariance(ParticleMoments m) {
    if(m.weight<=0.) return float3x3(0.);
    float3 mean=m.first/m.weight;
    return m.second/m.weight-float3x3(mean.x*mean,mean.y*mean,mean.z*mean);
}
// Symmetric Jacobi diagonalization. Scale first, so covariance units do not
// control convergence. Repeated eigenvalues need no arbitrary eigenvector tie-break.
void particle_eigen(float3x3 covariance,out float3 values,out float3x3 axes) {
    axes=float3x3(1.,0.,0.,0.,1.,0.,0.,0.,1.);
    float3 row=max(abs(covariance[0]),max(abs(covariance[1]),abs(covariance[2])));
    float scale=max(row.x,max(row.y,row.z));
    if(scale<=0.) {values=0.;return;}
    float3x3 a=covariance/scale;
    for(uint sweep=0;sweep<8;sweep++) {
        for(uint pair=0;pair<3;pair++) {
            uint p=pair==2u ? 1u : 0u,q=pair==0u ? 1u : 2u;
            float off=a[p][q];
            if(abs(off)<=1e-7) continue;
            float tau=(a[q][q]-a[p][p])/(2.*off);
            float t=(tau<0. ? -1. : 1.)/(abs(tau)+sqrt(1.+tau*tau));
            float c=rsqrt(1.+t*t),s=t*c;
            float3x3 j=float3x3(1.,0.,0.,0.,1.,0.,0.,0.,1.);
            j[p][p]=c;j[q][q]=c;j[p][q]=s;j[q][p]=-s;
            a=mul(transpose(j),mul(a,j));axes=mul(axes,j);
        }
    }
    values=max(float3(a[0][0],a[1][1],a[2][2])*scale,0.);
}
float3x3 particle_metric(ParticleMoments moments,float radius,float neighbourhood_radius,uint minimum_neighbours,float maximum_ratio,out float support) {
    // Yu–Turk Eq. 15: sparse covariance uses kn=0.5, not full support.
    support=.5*radius;
    float3x3 identity=float3x3(1.,0.,0.,0.,1.,0.,0.,0.,1.);
    if(moments.neighbours<minimum_neighbours) return identity/support;
    float3 values;float3x3 axes;particle_eigen(particle_covariance(moments),values,axes);
    float largest=max(values.x,max(values.y,values.z));
    if(largest<=0.) return identity/support;
    // E[x*x] for the radial weight 1-(r/R)^3 in a uniform 3D ball is .15 R².
    // This dimensionless normalization replaces the paper's scene-unit ks.
    float3 lengths=radius*max(values,largest/maximum_ratio)/(.15*neighbourhood_radius*neighbourhood_radius);
    // Blend through the neighbour threshold to avoid a discontinuous surface
    // when a single neighbour enters or leaves the fitting ball.
    float confidence=smoothstep(float(minimum_neighbours),2.*float(minimum_neighbours),float(moments.neighbours));
    lengths=lerp(float3(.5*radius),lengths,confidence);
    support=max(lengths.x,max(lengths.y,lengths.z));
    float3x3 diagonal=float3x3(1./lengths.x,0.,0.,0.,1./lengths.y,0.,0.,0.,1./lengths.z);
    return mul(axes,mul(diagonal,transpose(axes)));
}
float particle_cubic(float q) {
    if(q>=1.) return 0.;
    return q<=.5 ? 6.*q*q*q-6.*q*q+1. : 2.*(1.-q)*(1.-q)*(1.-q);
}
float particle_kernel(float3 relative,float3x3 metric) {
    return (8./3.141592653589793)*abs(determinant(metric))*particle_cubic(length(mul(metric,relative)));
}
