#define CONTROL_SAMPLES 4
#define CONTROL_TEXTURE Texture2DMS
#ifdef VORK_METAL
#define CONTROL_LOAD(tex, p, sample) load_ms(tex, p, sample)
#else
#define CONTROL_LOAD(tex, p, sample) tex.Load(p, sample)
#endif
// Metal's texture2d_ms::get_width/get_height take no mip argument, which GetDimensions emits.
#ifdef VORK_METAL
uint3 ms_dimensions(Texture2DMS<float> t) { __intrinsic_asm "uint3($0.get_width(), $0.get_height(), $0.get_num_samples())"; }
#define CONTROL_DIMENSIONS(tex, w, h) let dims_=ms_dimensions(tex); w=dims_.x; h=dims_.y; uint samples=dims_.z
#else
#define CONTROL_DIMENSIONS(tex, w, h) uint samples; tex.GetDimensions(w, h, samples)
#endif
#include "control_resolve.slangh"
