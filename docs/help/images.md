IMAGES
Use color_space Non-Color for data PNGs. Nested camera textures inherit shot profile
and timing.
Missing sampled image files, including environment images, use one magenta texel
and emit a warning naming the file to restore. Corrupt images, IES profiles and
colour-transform library data remain errors. Movie plates and outputs are video-only;
add sound to the rendered movie in an external editor.
16-bit integer and floating-point image inputs avoid block compression but use half-float texture precision; exact source integer codes are not preserved.

<!-- all -->
ANIMATED TEXTURE REFERENCE
image.file accepts .mov/.mp4/.mkv and %d/%04d frame patterns. Video requires
user-installed ffmpeg and ffprobe. image.time is seconds into the clip; omitted
follows scene time. The ordinary animation path ["images",name,"time"] remaps it.
Sequence time zero is the first on-disk frame. EXR sequences use framesPerSecond
from the first file when present; it must be finite and positive. Without that
metadata, sequences use render.fps. Interior
gaps are errors naming missing frames. Video uses displayed presentation timestamps
relative to the first displayed frame, honours edit lists and rotation, and holds
the last frame at or before time. All footage holds end frames outside its range.

Surface and environment footage follows the transport sample clock. Final
compositor images follow the nominal output-frame clock after exposure integration.

image.render captures use each parent transport sample's pose and time, with a
closed capture shutter. Captures retain scene dependencies even when the final
compositor replaces camera beauty. File/video compositor samples are premultiplied
before filtering; return premultiplied RGBA from custom composites.
