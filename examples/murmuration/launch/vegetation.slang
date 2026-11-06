// The source glTF omits its published leaf alpha. Bind that CC0 map explicitly.
import vork;
import blender.nodes;
struct Inputs { Image albedo; Image normal_map; Image opacity; };
Closure surface(ShadingContext c, Inputs i) {
    Closure m={};
    m.base_color=sample(i.albedo,c.TEXCOORD_0,c).rgb;
    m.geometry_opacity=sample(i.opacity,c.TEXCOORD_0,c).r>=.4?1.:0.;
    m.geometry_normal=Normal_Map(1.,sample(i.normal_map,c.TEXCOORD_0,c).rgb,c);
    m.specular_roughness=.85;
    return m;
}
