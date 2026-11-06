LIGHTS
Area lights emit along local -Z; use target to aim them at the subject.
Use one environment light for sky or HDR lighting; RGB render.background is only a backdrop.
Without an environment light the world is dim grey (0.05).
std/environment_image takes an Image input. vork std environment preprocesses HDR
lighting into an environment and a sun; vork std sky provides a sky and sun.
<!-- all -->
LIGHTING REFERENCE
std/sky sun direction: [cos(el)*sin(rot),sin(el),-cos(el)*cos(rot)] (radians;
rot 0 = -Z, pi/2 = +X, matching Blender).
Use illuminates and shadowed_by for light linking; use shadowed_by: [] for a light without shadows.
Use std/path for refracted caustics. Final's glass shadows attenuate along straight
connections; roughness does not spread them with distance.
