void cloth_segments(float3 p,float3 q,float3 a,float3 b,out float s,out float t);
bool cloth_body_cross(float3 p,float3 q,float3 a,float3 b,float3 c,out float t,out float3 weights) {
 float3 d=q-p,e=b-a,f=c-a,h=cross(d,f);float det=dot(e,h);t=0;weights=0;
 if(abs(det)<1e-12)return false;
 float3 s=p-a;float u=dot(s,h)/det;if(u<0||u>1)return false;
 float3 k=cross(s,e);float v=dot(d,k)/det;if(v<0||u+v>1)return false;
 t=dot(f,k)/det;weights=float3(1-u-v,u,v);return t>=0&&t<=1;
}
// A refitted native triangle BVH; positions and world frames come from Geometry.
float cloth_box_distance(float3 p,float3 lo,float3 hi) {float3 d=max(max(lo-p,p-hi),float3(0));return dot(d,d);}
uint3 cloth_body_indices(uint f){return uint3(collider_triangles_load(f));}
void cloth_collider_update(uint lane,Inputs inputs,bool initial) {
 for(uint v=lane;v<COLLIDER_VERTICES;v+=64) {
  float3 p=geometry_to_world(inputs.body,geometry_POSITION_load(inputs.body,v));
  collider_previous_store(v,initial?p:collider_current_load(v));collider_current_store(v,p);
 }
 DeviceMemoryBarrierWithGroupSync();
 for(uint f=lane;f<BODY_LEAVES;f+=64) {
  float3 a=float3(1e30),b=a,c=a,oa=a,ob=a,oc=a;
  if(f<BODY_TRIANGLES) {
   uint3 ids=cloth_body_indices(f);a=collider_current_load(ids.x);b=collider_current_load(ids.y);c=collider_current_load(ids.z);
   oa=collider_previous_load(ids.x);ob=collider_previous_load(ids.y);oc=collider_previous_load(ids.z);
  }
  body_a_store(f,a);body_b_store(f,b);body_c_store(f,c);body_old_a_store(f,oa);body_old_b_store(f,ob);body_old_c_store(f,oc);
  body_lo_store(BODY_LEAVES+f,f<BODY_TRIANGLES?min(a,min(b,c)):float3(1e30));body_hi_store(BODY_LEAVES+f,f<BODY_TRIANGLES?max(a,max(b,c)):float3(-1e30));
 }
 DeviceMemoryBarrierWithGroupSync();
 for(uint v=lane;v<COLLIDER_VERTICES;v+=64) {
  float3 n=0;
  for(uint j=collider_offsets_load(v);j<collider_offsets_load(v+1);j++) {
   uint f=collider_faces_load(j);n+=cross(body_b_load(f)-body_a_load(f),body_c_load(f)-body_a_load(f));
  }
  collider_normal_store(v,cloth_unit(n));
 }
 DeviceMemoryBarrierWithGroupSync();
 for(uint first=BODY_LEAVES/2;first>0;first/=2) {
  for(uint node=first+lane;node<2*first;node+=64) {
   body_lo_store(node,min(body_lo_load(2*node),body_lo_load(2*node+1)));
   body_hi_store(node,max(body_hi_load(2*node),body_hi_load(2*node+1)));
  }
  DeviceMemoryBarrierWithGroupSync();
 }
}
void cloth_register(uint i,uint kind,uint4 key,uint4 ids,float4 weights,float4 plane,float3 motion);
void cloth_body_generate(uint i) {
 float3 p=snapshot_load(i);uint stack[32];uint top=1;stack[0]=1;float best=1e30;uint nearest_face=0;float3 near=p,bary=0,normal=float3(0,1,0);
 while(top>0) {
  uint node=stack[--top];if(cloth_box_distance(p,body_lo_load(node),body_hi_load(node))>best)continue;
  if(node<BODY_LEAVES){stack[top++]=2*node;stack[top++]=2*node+1;continue;}
  uint f=node-BODY_LEAVES;if(f>=BODY_TRIANGLES)continue;
  float3 a=body_a_load(f),b=body_b_load(f),c=body_c_load(f),w;float3 q=cloth_closest(p,a,b,c,w);float d=dot(p-q,p-q);
  if(d<best){best=d;near=q;bary=w;uint3 ids=cloth_body_indices(f);normal=cloth_unit(w.x*collider_normal_load(ids.x)+w.y*collider_normal_load(ids.y)+w.z*collider_normal_load(ids.z));nearest_face=f;}
 }
 float offset=dot(near,normal)+THICKNESS;
 if(dot(p,normal)<offset) {
  float3 previous=bary.x*body_old_a_load(nearest_face)+bary.y*body_old_b_load(nearest_face)+bary.z*body_old_c_load(nearest_face);
  cloth_register(i,2,uint4(i,nearest_face,0,0),uint4(i,0,0,0),float4(1,0,0,0),float4(normal,offset),near-previous);
 }
 // Traverse the swept one-ring for reciprocal body-vertex / cloth-face contacts.
 float3 lo=min(p,old_load(i)),hi=max(p,old_load(i));
 for(uint j=neighbour_offsets_load(i);j<neighbour_offsets_load(i+1);j++) {
  uint v=neighbours_load(j);lo=min(lo,min(snapshot_load(v),old_load(v)));hi=max(hi,max(snapshot_load(v),old_load(v)));
 }
 lo-=2*THICKNESS;hi+=2*THICKNESS;top=1;stack[0]=1;
 while(top>0) {
  uint node=stack[--top];if(any(body_hi_load(node)<lo)||any(body_lo_load(node)>hi))continue;
  if(node<BODY_LEAVES){stack[top++]=2*node;stack[top++]=2*node+1;continue;}
  uint g=node-BODY_LEAVES;if(g>=BODY_TRIANGLES)continue;
  float3 ba=body_a_load(g),bb=body_b_load(g),bc=body_c_load(g);uint3 body_ids=cloth_body_indices(g);
  for(uint j=neighbour_offsets_load(i);j<neighbour_offsets_load(i+1);j++) {
   uint other=neighbours_load(j),u=min(i,other),v=max(i,other);float3 x=snapshot_load(u),y=snapshot_load(v),weights;
   float t;
   if(cloth_body_cross(x,y,ba,bb,bc,t,weights)) {
    float3 q=weights.x*ba+weights.y*bb+weights.z*bc,n=cloth_unit(cross(bb-ba,bc-ba));
    float3 previous=weights.x*body_old_a_load(g)+weights.y*body_old_b_load(g)+weights.z*body_old_c_load(g);
    cloth_register(i,5,uint4(u,v,g,0),uint4(u,v,0,0),float4(1-t,t,0,0),float4(n,dot(q,n)+THICKNESS),q-previous);
   }
   for(uint k=0;k<3;k++) {
    uint a_id=min(body_ids[k],body_ids[(k+1)%3]),b_id=max(body_ids[k],body_ids[(k+1)%3]);
    float3 a=collider_current_load(a_id),b=collider_current_load(b_id);
    if(any(max(x,y)+2*THICKNESS<min(a,b))||any(max(a,b)+2*THICKNESS<min(x,y)))continue;
    float s,r;cloth_segments(x,y,a,b,s,r);float3 p=lerp(x,y,s),q=lerp(a,b,r),d=p-q;
    float l=length(d);if(l>=2*THICKNESS)continue;
    float3 hint=cloth_unit(lerp(collider_normal_load(a_id),collider_normal_load(b_id),r));
    float3 n=hint;if(l>1e-8){n=d/l;if(dot(d,hint)<0)n=-n;}
    if(dot(d,n)>=THICKNESS)continue;
    float3 previous=lerp(collider_previous_load(a_id),collider_previous_load(b_id),r);
    cloth_register(i,4,uint4(u,v,a_id,b_id),uint4(u,v,0,0),float4(1-s,s,0,0),float4(n,dot(q,n)+THICKNESS),q-previous);
   }
  }
  for(uint j=incident_offsets_load(i);j<incident_offsets_load(i+1);j++) {
   uint f=incidents_load(j);uint3 ids=uint3(triangles_load(3*f),triangles_load(3*f+1),triangles_load(3*f+2));
   float3 a=snapshot_load(ids.x),b=snapshot_load(ids.y),c=snapshot_load(ids.z);
   float3 flo=min(a,min(b,c))-2*THICKNESS,fhi=max(a,max(b,c))+2*THICKNESS;
   if(any(max(ba,max(bb,bc))<flo)||any(min(ba,min(bb,bc))>fhi))continue;
   for(uint k=0;k<3;k++) {
    uint body_vertex=body_ids[k];float3 q=collider_current_load(body_vertex),n=collider_normal_load(body_vertex),weights;float3 r=cloth_closest(q,a,b,c,weights),d=r-q;
    if(dot(d,n)<THICKNESS&&length(d)<2*THICKNESS) {
     float3 previous=collider_previous_load(body_vertex);
     cloth_register(i,3,uint4(f,body_vertex,0,0),uint4(ids,0),float4(weights,0),float4(n,dot(q,n)+THICKNESS),q-previous);
    }
   }
  }
 }
}
