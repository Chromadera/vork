float container(float3 p) {
    float2 q=float2(length(p.xz),p.y);float squared=4.*DOMAIN*DOMAIN;bool inside=false;
    for(uint i=0u,j=BOWL_PROFILE_COUNT-1u;i<BOWL_PROFILE_COUNT;j=i++) {
        float2 a=BOWL_PROFILE[j],b=BOWL_PROFILE[i],edge=b-a;
        // The profile's closing axis is internal to the solid of revolution.
        if(a.x!=0. || b.x!=0.) {float2 delta=q-a-edge*saturate(dot(q-a,edge)/dot(edge,edge));squared=min(squared,dot(delta,delta));}
        if((a.y>q.y)!=(b.y>q.y) && q.x<a.x+(q.y-a.y)*(b.x-a.x)/(b.y-a.y)) inside=!inside;
    }
    float distance=sqrt(squared),ceramic=inside ? -distance : distance;
    return -min(ceramic,p.y-TABLE_TOP);
}
float initial_field(float3 p) {return max(max(p.y+.29,length(p.xz)-.83),container(p));}
bool initial_marker(uint id,out float3 q) {
 uint3 c=uint3(id%markers_dims.x,(id/markers_dims.x)%markers_dims.y,id/(markers_dims.x*markers_dims.y));
 q=float3((float(c.x)+.5)*MARKER_SPACING-.832,-.875+(float(c.y)+.5)*MARKER_SPACING,(float(c.z)+.5)*MARKER_SPACING-.832);
 return length(q.xz)<=.818;
}
float3 initial_velocity(float3 p,float dx) {
        float3 initial=0.;
        for(int axis=0;axis<3;axis++) {
            float3 face=p;face[axis]-=.5*dx;
            float radius=sqrt(1.1*face.x*face.x+.9*face.z*face.z);
            float y=face.y+.4,radial_width=0.08*0.08,depth_width=.18*.18;
            float e=exp(-(radius-.26)*(radius-.26)/radial_width-y*y/depth_width);
            float angle=atan2(face.z,face.x),t=saturate((radius-.06)/.12);
            float lobes=.22*cos(5.*angle)+.12*sin(7.*angle+.4);
            float a=.95*(1.+lobes*t*t*(3.-2.*t));
            float radial_derivative=.95*lobes*6.*t*(1.-t)/.12;
            float3 curl=float3(2.*a*face.x*y/depth_width*e,
                (a*(2.-2.*radius*(radius-.26)/radial_width)+radius*radial_derivative)*e,
                2.*a*face.z*y/depth_width*e);
            initial[axis]=3.271*curl[axis];
        }
 return initial;
}
float initial_pressure(float3 p) {return initial_field(p)<0. ? GRAVITY*max(0.,-.29-p.y) : 0.;}
bool film_accept(float2 p) {return length(p)>BOWL_RADIUS;}
float film_height(float2 p) {return TABLE_TOP;}
// Boundary forcing may be animated by the simulation clock (e.g. authored swell).
float3 boundary_velocity(float3 p,float time,float3 v) {return v;}
