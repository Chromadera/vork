// Compile-time technique values; Rust owns the public defaults.
#ifndef VORK_VOLUME_SHADOWS
#define VORK_VOLUME_SHADOWS 0
#endif
#ifndef VORK_VOLUME_SHADOW_SAMPLES
#define VORK_VOLUME_SHADOW_SAMPLES 16
#endif
#ifndef VORK_VOLUME_LIGHT_CLAMP
#define VORK_VOLUME_LIGHT_CLAMP 0.0
#endif
public static const bool VOLUME_SELF_SHADOWS=VORK_VOLUME_SHADOWS!=0;
public static const uint VOLUME_SHADOW_SAMPLES=VORK_VOLUME_SHADOW_SAMPLES;
public static const float VOLUME_LIGHT_CLAMP=VORK_VOLUME_LIGHT_CLAMP;
