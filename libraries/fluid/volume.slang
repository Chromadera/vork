// Volume integration and publication must use the same clipped field. Otherwise
// regularized occupancy on the domain boundary is counted, then discarded.
float published_field(float source,float wall,float offset,int3 cell) {
    if(!isfinite(source) && asuint(source)!=0x7f800000u) return source;
    float shifted=source<2.*DOMAIN ? source+offset : source;
    float f=asuint(source)==0x7f800000u ? wall : max(shifted,wall);
    if(any(cell==int3(0)) || any(cell==int3(surface_dims)-1))
        f=max(f,(2.*DOMAIN)/float(surface_dims.x));
    return f;
}

// Exact X integral of a linear-over-linear Y occupancy on one sign interval.
float ratio_average(float n0,float d0,float n1,float d1) {
 float f0=d0>0. ? saturate(n0/d0) : 0.,f1=d1>0. ? saturate(n1/d1) : 0.;
 if(d0<=0.) return f1;if(d1<=0.) return f0;
 if(d0>d1) {float t=d0;d0=d1;d1=t;t=f0;f0=f1;f1=t;}
 float r=d1/d0,e=r-1.,w;
 if(e<.1) w=.5+e*(1./6.+e*(-1./12.+e*(1./20.+e*(-1./30.+e/42.))));
 else w=isfinite(r) ? r/e*(1.-log(r)/e) : 1.;
 return lerp(f0,f1,w);
}
float occupied_x(float a0,float a1,float b0,float b1) {
 float cuts[4];cuts[0]=0.;cuts[1]=1.;uint count=2;
 if(a0*a1<0.) cuts[count++]=a0/(a0-a1);
 if(b0*b1<0.) cuts[count++]=b0/(b0-b1);
 for(uint i=1;i<count;i++) for(uint j=i;j>0 && cuts[j]<cuts[j-1];j--) {float t=cuts[j];cuts[j]=cuts[j-1];cuts[j-1]=t;}
 float sum=0.;
 for(uint i=1;i<count;i++) {
  float lo=cuts[i-1],hi=cuts[i],mid=.5*(lo+hi);
  float a=lerp(a0,a1,mid),b=lerp(b0,b1,mid);
  if(a<=0. && b<=0.) {sum+=hi-lo;continue;}
  if(a>=0. && b>=0.) continue;
  float al=lerp(a0,a1,lo),ah=lerp(a0,a1,hi),bl=lerp(b0,b1,lo),bh=lerp(b0,b1,hi);
  sum+=(hi-lo)*(a<0. ? ratio_average(-al,bl-al,-ah,bh-ah) : ratio_average(-bl,al-bl,-bh,ah-bh));
 }
 return sum;
}
float occupied_cell(float f[8]) {
 // Split at edge zeros and changes in the order of the two X crossings.
 // Thus narrow corner parcels cannot fall between quadrature samples.
 float cuts[8];cuts[0]=0.;cuts[1]=1.;uint count=2;
 for(uint k=0;k<4;k++) if(f[k]*f[k+4]<0.) cuts[count++]=f[k]/(f[k]-f[k+4]);
 float a=f[4]-f[0],b=f[5]-f[1],c=f[6]-f[2],d=f[7]-f[3];
 float qa=a*d-b*c,qb=f[0]*d+a*f[3]-f[1]*c-b*f[2],qc=f[0]*f[3]-f[1]*f[2];
 float roots[2];uint nr=0;
 if(qa!=0.) {float disc=qb*qb-4.*qa*qc;if(disc>=0.) {float r=sqrt(disc);roots[nr++]=(-qb-r)/(2.*qa);roots[nr++]=(-qb+r)/(2.*qa);}}
 else if(qb!=0.) roots[nr++]=-qc/qb;
 for(uint k=0;k<nr;k++) if(roots[k]>0. && roots[k]<1.) cuts[count++]=roots[k];
 for(uint i=1;i<count;i++) for(uint j=i;j>0 && cuts[j]<cuts[j-1];j--) {float t=cuts[j];cuts[j]=cuts[j-1];cuts[j-1]=t;}
 static const float nodes[16]={.0052995325,.0277124885,.0671843988,.1222977958,.1910618778,.2709916112,.3591982246,.4524937451,.5475062549,.6408017754,.7290083888,.8089381222,.8777022042,.9328156012,.9722875115,.9947004675};
 static const float weights[16]={.0135762297,.0311267620,.0475792558,.0623144856,.0747979944,.0845782597,.0913017075,.0947253052,.0947253052,.0913017075,.0845782597,.0747979944,.0623144856,.0475792558,.0311267620,.0135762297};
 float sum=0.;
 for(uint i=1;i<count;i++) {
  float width=cuts[i]-cuts[i-1];if(width<=0.) continue;
  for(uint k=0;k<16;k++) {
   float z=cuts[i-1]+width*nodes[k];
   sum+=width*weights[k]*occupied_x(lerp(f[0],f[4],z),lerp(f[1],f[5],z),lerp(f[2],f[6],z),lerp(f[3],f[7],z));
  }
 }
 return sum;
}

