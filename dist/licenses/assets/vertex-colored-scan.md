# Vertex-coloured mosquito scan

Derived from the mosquito mesh in [Mosquito In Amber](https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/MosquitoInAmber).
Mosquito scan by Geoffrey Marchal; model by Loïc Norgeot for Sketchfab (2018),
real-time refraction conversion by Sketchfab (2019). Licensed [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).

Changes: isolated the scanned mosquito (14,536 vertices / 11,434 triangles),
centred and uniformly scaled it, sampled its base-colour image at the vertices,
decoded sRGB to linear and applied the source material factor. Added a synthetic
scalar roughness channel derived from green, to exercise a second named input.
The original amber/eclats geometry and image resources are not included.
`vertex-colored-scan.bin` contains packed little-endian buffers declared by the scene.

Source: https://raw.githubusercontent.com/KhronosGroup/glTF-Sample-Assets/main/Models/MosquitoInAmber/glTF-Binary/MosquitoInAmber.glb
Source SHA-256: 1c0b49000650e8a8c00d69bc8d64e46d33527871a47d74e4f2278887325cc35f
