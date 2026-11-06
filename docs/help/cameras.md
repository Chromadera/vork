CAMERAS
camera.material selects a ray program; omitted selects std/camera. Its numeric Inputs use material tracks.
std/camera follows Blender physical cameras: projection, fit, lens, sensor_width/height, ortho_scale, shift, clip_start/end, use_dof, focus_distance and aperture_* are Inputs.
focus_object supplies sampled camera-axis depth; target and transforms retain node semantics.
<!-- all -->
CAMERA PROGRAM REFERENCE
CameraRay t_min/t_max bound geometry visibility in metres along its normalized direction. Optional camera_calibration declares projective acceleration; omission declares none. See light-paths and camera-json.
