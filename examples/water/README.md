# Water burst

A lobed vortex launches a water crown from a tiled ceramic basin on a walnut
table. The initial impulse is authored; this is not a resolved rigid-body impact.
The solver, surface producer, conservative film and whitewater come from
`libraries/fluid`; `bowl.slang` owns only the container and authored initial impulse.
Run `vork help fluid --all` for the library contract.

Try a small **closed-shutter diagnostic** first. This preserves the full fluid
setup but turns off motion blur, avoiding repeated simulation replay during an
exposure. Generated paths point back to the checkout's assets and buffers.

```sh
python3 examples/water/prepare_snapshot.py /tmp/water-snapshot.json
vork render /tmp/water-snapshot.json --profile std/path --at 0 --out /tmp/water.png
```

This first still shows the initialized basin. Increase `--at` to inspect the
splash, allowing time to replay the fluid history.

Image resolution alone does not reduce the simulation grids or particle budget.
The normal motion-blurred scene is much more expensive; a small still can take
many minutes. This diagnostic does not use the full movie's exposure or sampling
budget, so it says nothing about full-render speed. The README still came from an
earlier build, and this example does not reproduce it exactly.

Run `python3 examples/water/make_scene.py --vork target/release/vork`
(requires NumPy) to regenerate the scene and its standard mesh buffers. Render the
native 1080p sequence with:

```
vork render examples/water/scene.json --profile std/path --out water-frames
ffmpeg -framerate 24 -i water-frames/%06d.png -c:v libx264 -crf 16 -pix_fmt yuv420p -movflags +faststart water.mp4
```

The basin is 30 cm across, on a walnut surface with 32 cm boards. The
experiment runs for 1.5 seconds at 24 fps and is encoded at 24 fps: real time.
The 180-degree shutter exposes for 1/48 second. The simulation advances at
1/960 second per tick: 20 ticks per exposure. Independent frame ranges replay the same clock.
This is not a high-speed-camera take. The movie uses `std/path` for reference
dielectric transport; `std/final` supports the field directly but approximates
its layered refraction and shadow transmission.

The pressure solve uses a 336×112×336 staggered MAC grid over a
1×0.333×1-metre domain (2.976 mm cells). Solver coordinates use one sixth
of a metre, with an explicit conversion in the fluid builder; time is seconds.
Gravity is 9.81 m/s², and surface tension divided by water density is
7.28e-5 m³/s². The authored vortex preserves the old experiment's Froude
number at the new scale. Geometry and camera positions convert to SI once;
wood grain and optical absorption stay in world metres. The solve includes
subcell ghost-fluid pressure boundaries, finite basin walls and Laplace pressure.
The domain extends beyond the basin so water can leave the rim. Markers carry
velocity and a local affine velocity field through
[APIC transfer](https://doi.org/10.1145/2766996). Quadratic B-splines transfer
momentum to the grid; red/black SOR projects pressure; weighted velocity moments
reconstruct each marker's velocity and affine rows. Midpoint grid advection moves
the markers. Detached water comes from those markers.

Pressure uses a coarse [Zhu–Bridson](https://doi.org/10.1145/1073204.1073298)
weighted-centre boundary. The visible 768×256×768 field uses
[Yu–Turk anisotropic reconstruction](https://faculty.cc.gatech.edu/~turk/my_papers/particle_surfaces_tog.pdf):
weighted covariance, regularized principal axes, local centre relocation,
normalized cubic kernels and a density isovalue of 0.6. Its nominal support is
5.83 mm, with a 2.5-support fitting neighbourhood, 2:1 axis regularization and
a half-radius spherical fallback below 25 neighbours. The fit blends to full
anisotropy between 25 and 50 neighbours. Sparse marker volume is bounded by the rest
spacing cubed so isolated markers do not acquire a full kernel volume; this bound
fades out in dense neighbourhoods to retain boundary density compensation.
Fitting and evaluation are separate dispatches over one retained generation.
The bounding sphere of each ellipsoid rejects only samples outside its compact
support. A scalar display-isovalue correction preserves regularized geometric
volume, using eight Newton updates per tick and the same domain-boundary
clipping as the published field. Its column sums use deterministic
staged reductions; the correction never
feeds back into pressure or marker advection. Marker slots are retained; landing markers transfer their volume to the floor film.

The anisotropic display field is independent of the pressure boundary. Sorted
integer linked bins make gathers deterministic with one link per marker. Domain
escapes invalidate publication. The implicit primitive intersects the fine field
directly and reconstructs continuous shading normals. State, scratch and fitted
kernels all use the retained simulation clock and seek replay.

Water has IOR 1.333 and depth-dependent absorption. The low-contrast tile pattern
makes refraction and the basin floor visible without dominating every ripple. A detailed CC0 forest environment supplies reflections, with
a compact sun for highlights. The table meets the basin's underside.

Contacting markers deposit their rest volume exactly once into a retained floor
film-depth grid, using bilinear weights that sum to one even at the basin
footprint. Floor contact begins at the actual outer wall; no dry exclusion ring
leaves resting beads beside the basin. Deposited markers no
longer participate in pressure, velocity transfer or surface reconstruction.
A conservative diffusion closure spreads the film on the wettable finish;
closed boundaries prevent flux into the basin footprint or out of the domain.
The film volume is included in the display-volume target, so deposition cannot
inflate the remaining bulk water. The table darkens and gains a water-like
coat according to physical film depth. This represents a subgrid film: it does
not resolve its height, contact angle, impact crowns or shallow-water momentum.
See [asset credits](../assets/README.md) for the unmodified HDR environment.

The library adds 8,192 secondary slots using Ihmsen-style trapped-air, crest and
energy potentials. Spray is ballistic, foam follows liquid, and bubbles rise under
drag. These are one-way opaque scattering proxies; they do not remove bulk volume
or resolve an air phase. Counter-based random births preserve replay and avoid a
regular secondary lattice. Full pools suppress births.

Limitations: uniform grids, fixed bulk particle population, no adaptive sheet
resampling, suspended turbidity scattering, resolved air entrainment
or two-way rigid-body coupling. The 2.72 mm capillary length is only about one
pressure cell, so true-scale coefficients do not establish converged breakup. Thin features below
the particle/grid spacing remain unresolved. APIC and particle-count conservation
do not prove local conservation. Intersections retain the field's trilinear
approximation, and shutter samples hold fixed ticks. Caustics rely on ordinary
path sampling; this example does not implement a dedicated caustic solver.

Computed secondary points request previous simulation poses as well as current
ones. With shutter sampling and large retained dense fields, this can cause
expensive replay. Closed-shutter snapshots are useful diagnostics but do not
measure the shipped motion-blurred frame cost.
