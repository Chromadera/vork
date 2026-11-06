# LTC tables

`amplitude.rgba32f` is 64×64 row-major RGBA32F, little endian, 65,536 bytes.
Amplitude XY/W match three.js r180; amplitude Z additionally holds the sheen data below.
Read with linear filtering and clamp-to-edge, without mips. UV is
`(float2(roughness, sqrt(1 - NdotV)) * 63 + 0.5) / 64`.

Source: [three.js r180](https://github.com/mrdoob/three.js/blob/r180/examples/jsm/lights/RectAreaLightTexturesLib.js),
SHA256 `8acc50043aebc3ff0f63e21607ef2394cda41d634f92093bcb7ad6f1d1e2eb11`. The original float32 data are kept here; upload converts to filterable RGBA16F once per device.
Both licence notices accompany redistributions.

Reference: Real-Time Polygonal-Light Shading with Linearly Transformed Cosines.
Eric Heitz, Jonathan Dupuy, Stephen Hill and David Neubelt.
ACM Transactions on Graphics (Proceedings of ACM SIGGRAPH 2016) 35(4), 2016.
[Project](https://eheitzresearch.wordpress.com/415-2/),
[upstream data](https://github.com/selfshadow/ltc_code/tree/master/fit/results/ltc.js).

The otherwise unused blue channel of `amplitude.rgba32f` packs three 32×32
coefficient planes for Zeltner, Burley & Chiang's "Practical Multiple-Scattering
Sheen Using Linearly Transformed Cosines" (2022): inverse LTC A at (0,0), B at
(32,0), and directional albedo at (0,32). The values retain the Apache-2.0 Cycles
`table_sheen_ltc` data from Blender commit
[`9e2066aef7ef`](https://github.com/blender/blender/blob/9e2066aef7ef/intern/cycles/scene/shader.tables).
Copyright 2011–2022 Blender Foundation; license in
[`dist/licenses/Blender-Cycles.txt`](../../../../dist/licenses/Blender-Cycles.txt).
Modified amplitude file SHA256:
`7de0c17aed79e9c1e2416aee0a65ce8f1892d7079e9c5ac10b340cd955b8fd54`.

Each tile is indexed by (view cosine, roughness) over [0,1], with texel-center
coordinates and linear filtering. GGX amplitude XY and horizon-clipping W retain
their original data. Zero entries mean no fit; interpolate, then disable.
