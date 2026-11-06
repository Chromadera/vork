// Optical boundary services shared by path, raster and hardware refraction.
// Geometry acquisition and accepted occupancy transitions belong to callers.
import closure;
import openpbr_core;
import vork_constants;
struct DielectricSegment { float3 color; float depth; float ior; };
struct DielectricInterface { DielectricSegment incident; DielectricSegment transmitted; };
// An escaping ray has no finite interior chord. Positive-depth extinction is
// integrated once per finite segment; depth-zero colour belongs to interfaces.
float3 segment_attenuation(DielectricSegment s,float distance) {
    return s.depth>0. && distance<REG_FP32_MAX ? transmission_attenuation(s.color,s.depth,distance) : float3(1.);
}
Closure dielectric_interface(Closure c,float incident_ior,float transmitted_ior) {
    c.specular_ior=transmitted_ior/incident_ior;
    c.thin_film_ior/=incident_ior;c.thin_film_thickness*=incident_ior;
    return c;
}
Closure exiting_closure(Closure c,float3 normal) {
    let eta=c.specular_ior;c.geometry_normal=normal;
    return dielectric_interface(c,eta,1.);
}
