// Seven nearest birds in the deterministic, bounded spatial sample.
struct Inputs { float cohesion=.65; float alignment=2.4; float separation=1.1; };
void compute(ComputeContext c, Inputs i) {
    uint id=c.id.x; float3 position=p_load(id), velocity=v_load(id);
    int3 cell=int3(clamp(floor(position+16.),0.,31.));
    float distances[7]; uint neighbours[7];
    for(uint k=0;k<7;k++) { distances[k]=1e20; neighbours[k]=id; }
    for(int z=-1;z<=1;z++) for(int y=-1;y<=1;y++) for(int x=-1;x<=1;x++) {
        int3 other=cell+int3(x,y,z);
        if(any(other<0)||any(other>=32)) continue;
        uint base=((other.z*32+other.y)*32+other.x)*4;
        for(uint j=0;j<4;j++) {
            uint key=grid_load(base+j); if(key==0xffffffffu) break;
            uint neighbour=key&0xfffffu; if(neighbour==id) continue;
            float3 delta=p_load(neighbour)-position; float d2=dot(delta,delta);
            if(d2>=distances[6]) continue;
            // Fixed insertion order also resolves distance ties deterministically.
            for(uint k=0;k<7;k++) if(d2<distances[k]) {
                float old=distances[k]; uint previous=neighbours[k];
                distances[k]=d2; neighbours[k]=neighbour; d2=old; neighbour=previous;
            }
        }
    }
    float3 centre=0, heading=0, repel=0; uint count=0;
    for(uint k=0;k<7;k++) if(neighbours[k]!=id) {
        float3 delta=p_load(neighbours[k])-position;
        centre+=delta; heading+=v_load(neighbours[k]);
        repel-=delta/max(dot(delta,delta),.035); count++;
    }
    float t=c.time;
    // A broad roost orbit keeps the flock airborne without a point spring collapse.
    float3 roost=float3(3.*sin(t*.43),1.5*sin(t*.61),2.*cos(t*.37));
    float3 offset=position-roost;
    float3 desired=float3(-offset.z*.48,0,offset.x*.48);
    desired+=float3(1.8*sin(position.y*.55+t*.9),
                    1.6*sin(position.x*.38-t*1.2),1.5*cos(position.x*.3+t*.7));
    float3 force=(desired-velocity)*.65;
    force-=offset*max(0.,length(offset)-10.)*.15;
    force.y+=-.22*position.y-.3*velocity.y; // soft altitude band, gravity/ lift
    if(count>0) force+=i.cohesion*centre/count+i.alignment*(heading/count-velocity)+i.separation*repel/count;
    // Sweeping predator: local avoidance divides the cloud, social steering rejoins it.
    float3 predator=float3(11.*sin(t*.72),4.*sin(t*1.1),7.*cos(t*.72));
    float3 escape=position-predator; float d2=dot(escape,escape);
    force+=escape*(22.*exp(-d2/12.)/sqrt(max(d2,.1)));
    float acceleration=length(force); force*=min(1.,8./max(acceleration,.001));
    velocity+=force*c.dt; float speed=length(velocity);
    if(speed>0) velocity*=clamp(speed,2.,6.)/speed;
    next_v_store(id,velocity); next_p_store(id,position+velocity*c.dt);
}
