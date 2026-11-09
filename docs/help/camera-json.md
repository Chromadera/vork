CAMERA JSON sidecar, OpenCV conventions.
<!-- all -->
version: integer; schema version 2
fps: number|rational string; output frames per second
K: mat3; perspective only; row-major OpenCV pinhole intrinsics in pixels; pixel centres at integers
R: mat3; row-major world-to-camera rotation; OpenCV +X right, +Y down, +Z forward
t: vec3; world-to-camera translation; metres; camera point = R * world point + t
dist: number[5]; OpenCV [k1,k2,p1,p2,k3]; always zero for the pinhole camera
image_size: uvec2; [width,height] pixels
orthographic: {height:number}; vork-specific extension; vertical extent in metres; no K
frame_start: integer; frame sequences only (absent for --at stills); absolute index of frames[0], so frames[i] is frame frame_start+i
frames: object[]; frame index/fps defines nominal output time; evaluated pose uses numeric seconds; constant fields at clip root, varying fields per frame

Uncalibrated camera programs omit K, dist and orthographic; depth is distance along the generated ray in metres (PNG depth unit: 1 metre).
