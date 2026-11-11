CAMERAS
camera.material selects a ray program; omitted selects std/camera. Its numeric Inputs use material tracks. For bundled optics, set program:"std/camera" explicitly; a material with no program selects std/openpbr. Custom camera programs are also supported (vork help light-paths).
std/camera follows Blender physical cameras. Input fields below use material.inputs; lens and sensor sizes are mm, clip/focus/ortho_scale metres, aperture_rotation radians. Exact lookup: vork help field std/camera.lens.
focus_object supplies sampled camera-axis depth; target and transforms retain node semantics.
For orthographic projection, ortho_scale is a full extent, not a half extent.
With aspect = width/height, Horizontal fit (fit:"horizontal") sets full width = ortho_scale
and full height = ortho_scale/aspect. Vertical fit (fit:"vertical") sets full height = ortho_scale
and full width = ortho_scale*aspect. Auto fit (fit:"auto") uses Horizontal when aspect > 1,
otherwise Vertical. Auto chooses from image aspect, not scene bounds.
<!-- all -->
CAMERA PROGRAM REFERENCE
Physical apertures can select sampled path transport where supported, retaining the selected profile's sample budget.
CameraRay t_min/t_max bound geometry visibility in metres along its normalized direction. Optional camera_calibration declares projective acceleration; omission declares none. See light-paths and camera-json.
