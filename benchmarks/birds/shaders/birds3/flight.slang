// Feathered flight mesh, articulated at the shoulder and wrist. Phase cohorts
// share deformation streams; time_offset keeps the flock out of lockstep.
import vork;
import blender.nodes;
struct Inputs {
    Image albedo;
    Image normal_map;
    float wing_start = .045;
    float flap_rate = 6.2;
    float flap_amount = 1.;
};
float3 displace(DisplacementContext c, Inputs i) {
    float3 p=c.POSITION;
    float span=max(0.,abs(p.x)-i.wing_start);
    float wing=smoothstep(i.wing_start,i.wing_start+.018,abs(p.x));
    // Several quick wingbeats followed by a short, level glide. Smooth gates
    // preserve continuous wing velocities at both transitions.
    float rhythm=frac(c.time/3.7);
    float active=1.-smoothstep(.65,.77,rhythm)+smoothstep(.92,1.,rhythm);
    float beat=sin(c.time*i.flap_rate*6.2831853);
    float angle=(.12+.78*beat)*active;
    float wrist=smoothstep(.08,.17,span);
    float a=angle-.22*wrist*sin(c.time*i.flap_rate*6.2831853-.7)*active;
    return float3(sign(p.x)*span*(cos(a)-1.),span*sin(a),-.025*wrist*beat*active)*wing*i.flap_amount;
}
Closure surface(ShadingContext c, Inputs i) {
    Closure m={};
    float4 tex=sample(i.albedo,c.TEXCOORD_0,c);
    m.base_color=tex.rgb*.55;m.geometry_opacity=tex.a>=.4?1.:0.;
    m.geometry_normal=Normal_Map(1.,sample(i.normal_map,c.TEXCOORD_0,c).rgb,c);
    m.specular_roughness=.62;
    m.specular_weight=.35;
    return m;
}
