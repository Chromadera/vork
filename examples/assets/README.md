# Animated character assets

Unmodified GLBs from KhronosGroup/glTF-Sample-Assets (downloaded 2026-10-21).
Both contain renderable meshes, skins and animation clips; no external buffers or textures are needed.

- [Fox](https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/Fox):
  model © 2014 PixelMannen, CC0 1.0; rigging and animation © 2014 tomkranis,
  CC-BY 4.0; glTF conversion © 2017 @AsoboStudio and @scurest, CC-BY 4.0.
  Clips: Survey, Walk, Run. Native dimensions are centimetre scale; the starter scales by 0.01.
- [CesiumMan](https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/CesiumMan):
  © 2017 Cesium, CC-BY 4.0. One walking clip (index 0).
  Cesium logo © 2015 Cesium; see the included legal-mark notice (no trademark rights granted).

Full licence texts: [CC-BY-4.0.txt](CC-BY-4.0.txt), [CC0-1.0.txt](CC0-1.0.txt),
[LicenseRef-LegalMark-Cesium.txt](LicenseRef-LegalMark-Cesium.txt).

scenes/assets/vertex-colored-scan.bin derives from Mosquito In Amber:
mosquito scan by Geoffrey Marchal, model by Loïc Norgeot for Sketchfab (2018),
refraction conversion by Sketchfab (2019), CC BY 4.0.
https://creativecommons.org/licenses/by/4.0/
See dist/licenses/assets/vertex-colored-scan.md for source and modifications.
- `brick.png`: 64×64 procedural brick pattern generated for `../programs.json`; CC0 1.0.
- `studio_small_09_1k.hdr`: [Studio Small 09](https://polyhaven.com/a/studio_small_09) by Sergej Majboroda,
  Poly Haven, CC0 1.0; unmodified 1k equirectangular HDR used by the environment and world-medium starters.

## Asset provenance

Dimensions below are approximate rest-mesh bounding extents in scene axes after
the GLB root transforms, before scene scaling or animation. Y is up.

| Asset | Dimensions X × Y × Z | Forward | Clips | Licence |
|---|---|---|---|---|
| Fox.glb | 25.19 × 79.03 × 154.72 native units; at starter scale 0.01: 0.252 × 0.790 × 1.547 m | +Z | 0 Survey, 1 Walk, 2 Run | model CC0; rig/animation/conversion CC-BY-4.0 (credits above) |
| CesiumMan.glb | 1.138 × 1.507 × 0.312 m | +Z | 0 unnamed walk | CC-BY-4.0, Cesium legal-mark notice |
| studio_small_09_1k.hdr | 1024 × 512 pixels, equirectangular | environment longitude; rotate about Y to place highlights | none | CC0, Sergej Majboroda / Poly Haven |
| brick.png | 64 × 64 pixels | UV image | none | CC0 |
| medium-spots.gltf | punctual spot light; no mesh extent | local −Z | none | LicenseRef-vork-Preview (vork; see root LICENSE) |
| starter-geometry.bin, geometry.bin | scene-defined geometry buffers; dimensions set by each scene | scene axes | none | LicenseRef-vork-Preview (vork; see root LICENSE) |

Retain the licence files in this directory when copying assets.

- `forest_slope_1k.hdr`: [Forest Slope](https://polyhaven.com/a/forest_slope) by Andreas Mischok,
  Poly Haven, [CC0 1.0](https://polyhaven.com/license); unmodified 1024 × 512 equirectangular HDR
  used for the water example's reflected environment and illumination. No physical texel scale.
