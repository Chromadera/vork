import vork;
import blender.nodes;
struct Inputs { Image albedo; Image normal_map; };
Closure surface(ShadingContext c, Inputs i) {
    Closure m={};m.base_color=sample(i.albedo,c.TEXCOORD_0,c).rgb;
    m.specular_roughness=.42;
    m.geometry_normal=Normal_Map(1.,sample(i.normal_map,c.TEXCOORD_0,c).rgb,c);
    return m;
}
