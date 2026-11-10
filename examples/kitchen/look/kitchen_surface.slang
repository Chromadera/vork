import vork;
import blender.procedurals;
import blender.texture_coordinate;
struct Inputs {
 float3 base_a=float3(.8),base_b=float3(.8);float roughness=.5,metalness=0.,ior=1.45,specular_weight=1.,transmission=0.;
 float3 emission=float3(0.);float emission_strength=0.,coat=0.,coat_roughness=.03,opacity=1.,sss=0.,sss_radius=.05;float3 sss_scale=float3(1.,.2,.1);
 int wood_chain=0;float wood_scale=11.2,wood_detail=1.,wood_roughness=.25;
 int use_noise=0,use_image=0,coord_mode=0;float noise_scale=5.,noise_detail=2.,noise_roughness=.5,noise_distortion=0.;
 float4 map0=float4(1,0,0,0),map1=float4(0,1,0,0),map2=float4(0,0,1,0);Image color_map; Image palette; int use_palette=0,gradient_mode=0;
};
Closure surface(ShadingContext ctx,Inputs i) {
 Closure c={};float3 color=i.base_a;
 if(i.use_noise!=0) {
  let coord=texture_coordinate(ctx);float3 q=i.coord_mode==0?float3(coord.Generated.x,1.-coord.Generated.z,coord.Generated.y):float3(coord.Object.x,-coord.Object.z,coord.Object.y);
  float4 p=float4(q,1.);float3 v=float3(dot(p,i.map0),dot(p,i.map1),dot(p,i.map2));
  if(i.wood_chain!=0){float first=noise(v,i.wood_scale,i.wood_detail,i.wood_roughness,2.,0.);v=float3(first);}
  float t=noise(v,i.noise_scale,i.noise_detail,i.noise_roughness,2.,i.noise_distortion);
  if(i.gradient_mode==1)t=v.x;
  if(i.gradient_mode==2)t=max(v.x,0.)*max(v.x,0.);
  if(i.gradient_mode==3){float x=saturate(v.x);t=x*x*(3.-2.*x);}
  if(i.gradient_mode==4)t=(v.x+v.y)*.5;
  if(i.gradient_mode==5)t=atan2(v.y,v.x)/6.28318530718+.5;
  if(i.gradient_mode>=6){t=max(1.-length(v),0.);if(i.gradient_mode==7)t*=t;}
  color=i.use_palette!=0?sample(i.palette,float2((saturate(t)*255.+.5)/256.,.5),ctx).rgb:lerp(i.base_a,i.base_b,saturate(t));
 }
 if(i.use_image!=0) color*=sample(i.color_map,ctx.TEXCOORD_0,ctx).rgb;
 c.base_color=color;c.base_metalness=i.metalness;c.specular_roughness=i.roughness;c.specular_ior=i.ior;c.specular_weight=i.specular_weight;
 c.transmission_weight=i.transmission;c.transmission_color=sqrt(max(color,float3(0.)));c.geometry_opacity=i.opacity;
 c.coat_weight=i.coat;c.coat_roughness=i.coat_roughness;c.emission_color=i.emission;c.emission_luminance=i.emission_strength;
 c.subsurface_weight=i.sss;c.subsurface_color=color;c.subsurface_radius=i.sss_radius;c.subsurface_radius_scale=i.sss_scale;
 return c;
}
