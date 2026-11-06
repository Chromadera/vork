#define CONTROL_SAMPLES 1
#define CONTROL_TEXTURE Texture2D
#define CONTROL_LOAD(tex, p, sample) tex.Load(int3(p, 0))
#define CONTROL_DIMENSIONS(tex, w, h) tex.GetDimensions(w, h)
#include "control_resolve.slangh"
