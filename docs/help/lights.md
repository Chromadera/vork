LIGHTS
Area lights emit along local -Z; use target to aim them at the subject.
Use one environment light for sky or HDR lighting; RGB render.background is only a backdrop.
Without an environment light the world is dim grey (0.05).
std/environment_image takes an Image input. vork std environment preprocesses HDR
lighting into an environment and a sun; vork std sky provides a sky and sun.
For `vork std environment`, fps is a positive number or rational string (e.g.
"30000/1001"). Use a separate output directory/stem: generated EXR frames must
not alias input frames, including through symlinks. Invalid rates and output/source
collisions fail before frame writes. SDK helper fps options remain numeric.
DAYLIGHT STARTING RECIPE
Build sky and sun with `vork std sky '{"name":"daylight"}'` and merge the
returned fragment into your scene (keep both the environment and solar light).
In render, set "display":{"program":"std/agx","inputs":{"exposure":-3}}.
About -3 stops is a daylight starting point, not a universal exposure; there is
no implicit auto-exposure. Render a representative frame, then read the stdout
`exposure:` summary (clipped, near-black and mean; also in meta.json exposure).
Inspect highlights and shadows in the image and adjust render.display.inputs.exposure
for the shot. Keep the physical sun energy while choosing the display exposure.
<!-- all -->
LIGHTING REFERENCE
std/sky sun direction: [cos(el)*sin(rot),sin(el),-cos(el)*cos(rot)] (radians;
rot 0 = -Z, pi/2 = +X, matching Blender).
Use illuminates and shadowed_by for light linking; use shadowed_by: [] for a light without shadows.
Use std/path for refracted caustics. Final's glass shadows attenuate along straight
connections; roughness does not spread them with distance.
