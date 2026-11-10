// Original XPBD cloth. Configuration, typed buffers and callbacks precede this include.
groupshared float3 cloth_x[CLOTH_VERTICES];
void cloth_contacts_gauss_seidel();
float3 cloth_unit(float3 x) {float l=length(x);return l>1e-12?x/l:float3(0,1,0);}
float3 cloth_closest(float3 p,float3 a,float3 b,float3 c,out float3 bary) {
 float3 ab=b-a,ac=c-a,ap=p-a;float d1=dot(ab,ap),d2=dot(ac,ap);
 if(d1<=0&&d2<=0){bary=float3(1,0,0);return a;}
 float3 bp=p-b;float d3=dot(ab,bp),d4=dot(ac,bp);
 if(d3>=0&&d4<=d3){bary=float3(0,1,0);return b;}
 float vc=d1*d4-d3*d2;
 if(vc<=0&&d1>=0&&d3<=0){float v=d1/(d1-d3);bary=float3(1-v,v,0);return a+v*ab;}
 float3 cp=p-c;float d5=dot(ab,cp),d6=dot(ac,cp);
 if(d6>=0&&d5<=d6){bary=float3(0,0,1);return c;}
 float vb=d5*d2-d1*d6;
 if(vb<=0&&d2>=0&&d6<=0){float w=d2/(d2-d6);bary=float3(1-w,0,w);return a+w*ac;}
 float va=d3*d6-d5*d4;
 if(va<=0&&(d4-d3)>=0&&(d5-d6)>=0){float w=(d4-d3)/((d4-d3)+(d5-d6));bary=float3(0,1-w,w);return b+w*(c-b);}
 float denom=va+vb+vc;
 if(abs(denom)<1e-20){bary=float3(1,0,0);return a;}
 float v=vb/denom,w=vc/denom;bary=float3(1-v-w,v,w);return a+v*ab+w*ac;
}
void cloth_stretch(uint k,float h) {
 uint a=edges_load(2*k),b=edges_load(2*k+1);float3 d=cloth_x[b]-cloth_x[a];float l=length(d);
 float wa=mass_load(a),wb=mass_load(b),alpha=stretch_compliance_load(k)/(h*h);
 if(l<1e-12||wa+wb==0)return;
 float old=lambda_load(k),dl=(-(l-rest_load(k))-alpha*old)/(wa+wb+alpha);
 lambda_store(k,old+dl);float3 q=d*(dl/l);cloth_x[a]-=wa*q;cloth_x[b]+=wb*q;
}
void cloth_bend(uint k,float h) {
 uint a=hinges_load(4*k),b=hinges_load(4*k+1),c=hinges_load(4*k+2),d=hinges_load(4*k+3);
 float3 e=cloth_x[b]-cloth_x[a],u=cloth_x[c]-cloth_x[a],v=cloth_x[d]-cloth_x[a];
 float3 n1=cross(e,u),n2=cross(v,e);float e2=dot(e,e),n12=dot(n1,n1),n22=dot(n2,n2);
 if(min(e2,min(n12,n22))<1e-16)return;
 float el=sqrt(e2);float angle=atan2(dot(cross(n1/sqrt(n12),n2/sqrt(n22)),e/el),dot(n1,n2)/sqrt(n12*n22));
 float error=angle-bend_rest_load(k);error-=6.28318530718*floor((error+3.14159265359)/6.28318530718);
 float3 gc=-el*n1/n12,gd=-el*n2/n22;float tc=dot(u,e)/e2,td=dot(v,e)/e2;
 float3 ga=-(1-tc)*gc-(1-td)*gd,gb=-tc*gc-td*gd;
 float wa=mass_load(a),wb=mass_load(b),wc=mass_load(c),wd=mass_load(d),alpha=bend_compliance_load(k)/(h*h);
 float den=wa*dot(ga,ga)+wb*dot(gb,gb)+wc*dot(gc,gc)+wd*dot(gd,gd)+alpha;
 if(den<=0)return;
 float old=bend_lambda_load(k),dl=(-error-alpha*old)/den;bend_lambda_store(k,old+dl);
 cloth_x[a]+=wa*dl*ga;cloth_x[b]+=wb*dl*gb;cloth_x[c]+=wc*dl*gc;cloth_x[d]+=wd*dl*gd;
}
// Structural colours stay inside one workgroup; contacts use the whole GPU.
void cloth_integrate(ComputeContext ctx,Inputs inputs) {
 if(ctx.dt>0&&contact_status_load(0)>0)return;
 uint i=ctx.id.x;float h=ctx.dt;
 float3 p=h==0?geometry_POSITION_load(i):p_load(i),v=h==0?float3(0):velocity_load(i);
 old_store(i,p);
 v=exp(-DAMPING*h)*v+cloth_force(i,p,v,ctx.time)*h;
 float3 predicted=mass_load(i)==0?geometry_POSITION_load(i):p+v*h;
 velocity_store(i,v);predicted_store(i,predicted);p_store(i,predicted);
 for(uint k=i;k<CLOTH_EDGES;k+=CLOTH_VERTICES)lambda_store(k,0);
 for(uint k=i;k<CLOTH_HINGES;k+=CLOTH_VERTICES)bend_lambda_store(k,0);
}
void cloth_solve(ComputeContext ctx,Inputs inputs) {
 if(contact_status_load(0)>0)return;
 uint lane=ctx.id.x;bool has_contacts=false;
 if(lane==0)for(uint i=0;i<CLOTH_VERTICES;i++)if(contact_count_load(i)>0){has_contacts=true;break;}
 for(uint i=lane;i<CLOTH_VERTICES;i+=64)cloth_x[i]=p_load(i);
 GroupMemoryBarrierWithGroupSync();
 for(uint iteration=0;iteration<CLOTH_ITERATIONS/CLOTH_ROUNDS;iteration++) {
  for(uint color=0;color<CLOTH_EDGE_COLORS;color++) {
   for(uint k=edge_colors_load(color)+lane;k<edge_colors_load(color+1);k+=64)cloth_stretch(k,ctx.dt);
   GroupMemoryBarrierWithGroupSync();
  }
  for(uint color=0;color<CLOTH_BEND_COLORS;color++) {
   for(uint k=bend_colors_load(color)+lane;k<bend_colors_load(color+1);k+=64)cloth_bend(k,ctx.dt);
   GroupMemoryBarrierWithGroupSync();
  }
  if(lane==0&&has_contacts)cloth_contacts_gauss_seidel();
  GroupMemoryBarrierWithGroupSync();
 }
 for(uint i=lane;i<CLOTH_VERTICES;i+=64)p_store(i,cloth_x[i]);
}
void cloth_publish(ComputeContext ctx,Inputs inputs) {
 uint i=ctx.id.x;float3 p=p_load(i),n=0,t=0,b=0;
 for(uint j=incident_offsets_load(i);j<incident_offsets_load(i+1);j++) {
  uint f=incidents_load(j);uint3 ids=uint3(triangles_load(3*f),triangles_load(3*f+1),triangles_load(3*f+2));
  float3 e1=p_load(ids.y)-p_load(ids.x),e2=p_load(ids.z)-p_load(ids.x);
  n+=cross(e1,e2);
  float2 u=cloth_uv(ids.y)-cloth_uv(ids.x),v=cloth_uv(ids.z)-cloth_uv(ids.x);float det=u.x*v.y-u.y*v.x;
  if(CLOTH_HAS_UV&&abs(det)>1e-12){t+=(v.y*e1-u.y*e2)/det;b+=(-v.x*e1+u.x*e2)/det;}else t+=e1;
 }
 n=cloth_unit(n);t-=dot(t,n)*n;if(dot(t,t)<1e-16)t=cross(abs(n.y)<.9?float3(0,1,0):float3(1,0,0),n);
 normal_store(i,n);tangent_store(i,float4(cloth_unit(t),dot(cross(n,t),b)<0?-1.:1.));
 // Preserve integrated velocity: subtracting two world positions each tick
 // loses small accelerations at metre-scale origins. Only projection corrects v.
 velocity_store(i,ctx.dt>0?(mass_load(i)==0?(p-old_load(i))/ctx.dt:velocity_load(i)+(p-predicted_load(i))/ctx.dt):float3(0));
}
