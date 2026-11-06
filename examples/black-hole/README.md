# Authored Schwarzschild propagation

From the checkout root, generate the disk and debris buffers, then try a small still:

```sh
python3 examples/black-hole/generate.py
vork render examples/black-hole/scene.json --profile std/path --at 6 --set resolution=320x180 --out /tmp/black-hole.png
vork render examples/black-hole/scene.json --profile std/path --out /tmp/black-hole.mp4
```

The shot is twelve seconds at 1080p; `std/path` is required. The generated disk
and debris buffers stay outside Git. For source builds, retain the pinned
`VORK_SLANGC` environment described in the build instructions.

`propagation.slang` owns the Schwarzschild optical metric in isotropic
coordinates (G=c=M=1), polygonal integration, capture and bolometric redshift.
The renderer traces ordinary straight segments. The disk, moon, ship and dust
are geometry, and the sky uses the escaping direction.

This is a nonrotating metric and artistic disk, without retarded animation or
spectral frequency shifting. Step size is an authored accuracy/cost choice.
Launch-direction light sampling can be noisy for small emitters.

## Ship credit

**Corinth Cargo Transporter**, by **Yvo Pors**, licensed **CC BY 4.0**.
[Original model](https://sketchfab.com/3d-models/corinth-cargo-transporter-7614d85856f6477ba2a603970dcbb0dd),
[license](https://creativecommons.org/licenses/by/4.0/).
The decimated glTF and resized textures are under `assets/corinth`, with the
original license and a description of the modifications. The complete ship is
about 2.3 MB. This scene changes its roughness, scale, orientation and placement.
