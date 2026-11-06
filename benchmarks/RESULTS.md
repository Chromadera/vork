# Benchmark results

Measured renderer: `81ae2b0755bb696fc3e904744b76a103feded023`.
Linux NVIDIA L4, four CPUs, 32 GiB per worker; seed 11.
Binary SHA-256: `e4d4e5fb585de30ea7f9e69e874ddee1951e046a6dc9441ee1563c90bfe5e115`.

Raw logs, measurements and contact sheets are not in this repository.
Ranges are observed minima/maxima, not confidence intervals. See the README
for timing boundaries, quality differences, limitations and rerun recipes.
New final uses eight samples + OIDN; draft four + OIDN. Preview and path
results are carried forward and were not remeasured on this build.

## New std/final vs EEVEE — seven full movies

| Scene | Warm final s (measured) | First after setup s | Setup s | EEVEE s (reused measured) | Comparison | Prior 32-sample final s | Change in command time |
|---|---|---|---|---|---|---|---|
| D-cube | 35.125 [34.900–35.458], n=3 | 48.345 | 244.743 | 48.140 | 1.37× faster | 40.460 [40.437–40.712], n=3 | -13.2% time |
| B-blocking | 47.082 [46.812–47.339], n=3 | 61.530 | 251.503 | 88.072 | 1.87× faster | 67.901 [67.808–67.924], n=3 | -30.7% time |
| B-interior | 59.231 [58.704–59.918], n=3 | 66.040 | 164.911 | 107.026 | 1.81× faster | 127.379 [126.275–127.648], n=3 | -53.5% time |
| B-car-pan | 81.740 [81.492–81.788], n=3 | 95.032 | 243.761 | 93.489 | 1.14× faster | 185.242 [185.241–185.304], n=3 | -55.9% time |
| B-night | 92.201 [91.906–92.917], n=3 | 98.678 | 162.934 | 213.477 | 2.32× faster | 270.377 [270.323–270.477], n=3 | -65.9% time |
| C-ToyCar | 41.417 [41.003–41.770], n=3 | 103.972 | 200.125 | 81.295 | 1.96× faster | 74.417 [74.350–75.187], n=3 | -44.3% time |
| C-SheenChair | 37.855 [37.751–38.309], n=3 | 47.233 | 163.350 | 61.211 | 1.62× faster | 55.193 [54.852–55.284], n=3 | -31.4% time |

240 frames, 1920×1080, 24 fps. EEVEE/vork >1 means vork faster. Quality and timing boundaries differ.

## Per-scene final stills against retained EEVEE

| Scene | Warm final s (measured) | First after setup s | Setup s | EEVEE s (reused measured) | Comparison | Prior 32-sample final s |
|---|---|---|---|---|---|---|
| B-blocking | 2.094 [1.993–2.296], n=5 | 2.298 | 250.792 | 2.405 | 1.15× faster | 1.990 [1.988–2.040], n=5 |
| B-push-in | 1.993 [1.940–2.043], n=5 | 2.348 | 246.683 | 2.912 | 1.46× faster | 2.245 [2.143–2.343], n=5 |
| B-turntable | 2.247 [2.101–2.401], n=5 | 3.817 | 251.239 | 3.148 | 1.40× faster | 2.302 [2.243–2.349], n=5 |
| B-walking | 1.890 [1.840–2.047], n=5 | 3.615 | 238.026 | 2.612 | 1.38× faster | 1.848 [1.839–1.992], n=5 |
| B-interior | 1.738 [1.687–1.789], n=5 | 1.992 | 192.702 | 2.892 | 1.66× faster | 2.245 [2.196–2.295], n=5 |
| B-street | 2.043 [1.992–2.149], n=5 | 2.400 | 239.326 | unmeasured | unmeasured | 1.841 [1.838–1.945], n=5 |
| B-rack-focus | 1.787 [1.735–1.791], n=5 | 82.057 | 261.914 | unmeasured | unmeasured | 1.738 [1.688–1.786], n=5 |
| B-car-pan | 1.691 [1.634–1.936], n=5 | 1.891 | 175.944 | 2.458 | 1.45× faster | 2.497 [2.495–2.543], n=5 |
| B-crowd | 2.046 [1.997–2.148], n=5 | 2.393 | 264.350 | unmeasured | unmeasured | 2.194 [2.141–2.196], n=5 |
| B-night | 1.937 [1.892–1.951], n=5 | 2.141 | 162.107 | 2.963 | 1.53× faster | 2.904 [2.899–2.956], n=5 |
| C-Sponza | unmeasured | licence hold | licence hold | unmeasured | unmeasured | unmeasured |
| C-ABeautifulGame | 1.887 [1.836–1.895], n=5 | 3.361 | 155.367 | unmeasured | unmeasured | 2.650 [2.549–2.701], n=5 |
| C-SheenChair | 1.635 [1.585–1.639], n=5 | 3.256 | 157.395 | 3.114 | 1.90× faster | 2.095 [2.001–2.146], n=5 |
| C-ToyCar | 2.043 [1.990–2.086], n=5 | 3.661 | 241.043 | 3.271 | 1.60× faster | 2.297 [2.246–2.299], n=5 |
| C-Lantern | 1.587 [1.585–1.644], n=5 | 3.202 | 163.488 | unmeasured | unmeasured | 2.398 [2.248–2.595], n=5 |
| C-ClearcoatWicker | 1.938 [1.887–1.992], n=5 | 3.609 | 240.495 | unmeasured | unmeasured | 2.395 [2.299–2.452], n=5 |
| C-GlassVaseFlowers | 2.451 [2.395–2.551], n=5 | 24.976 | 242.019 | unmeasured | unmeasured | 4.271 [4.169–4.276], n=5 |
| C-GlamVelvetSofa | 1.739 [1.735–1.791], n=5 | 3.507 | 225.740 | unmeasured | unmeasured | 2.143 [1.989–2.198], n=5 |
| C-StainedGlassLamp | 2.192 [2.142–2.198], n=5 | 52.351 | 178.046 | unmeasured | unmeasured | 3.258 [3.256–3.268], n=5 |
| C-IridescenceLamp | 1.738 [1.682–1.741], n=5 | 2.750 | 158.812 | unmeasured | unmeasured | 2.396 [2.297–2.448], n=5 |
| C-AnisotropyBarnLamp | 1.788 [1.736–1.794], n=5 | 3.005 | 158.528 | unmeasured | unmeasured | 2.248 [2.245–2.300], n=5 |
| C-CommercialRefrigerator | 1.993 [1.942–1.997], n=5 | 3.963 | 230.893 | unmeasured | unmeasured | 1.991 [1.940–2.041], n=5 |
| C-CesiumMilkTruck | 1.583 [1.538–1.636], n=5 | 3.008 | 165.513 | unmeasured | unmeasured | 1.741 [1.690–1.741], n=5 |
| C-RecursiveSkeletons | 1.785 [1.739–1.797], n=5 | 2.195 | 231.343 | unmeasured | unmeasured | 2.094 [2.042–2.199], n=5 |
| C-MorphStressTest | 1.991 [1.939–2.093], n=5 | 3.659 | 233.305 | unmeasured | unmeasured | 1.841 [1.790–1.849], n=5 |
| C-NodePerformanceTest | 2.602 [2.545–2.703], n=5 | 20.806 | 157.678 | unmeasured | unmeasured | 2.898 [2.856–2.917], n=5 |
| D-empty | 2.096 [1.941–2.252], n=5 | 2.097 | 242.738 | unmeasured | unmeasured | 1.738 [1.690–1.789], n=5 |
| D-cube | 1.584 [1.535–1.637], n=5 | 1.790 | 163.332 | 2.608 | 1.65× faster | 1.742 [1.689–1.791], n=5 |
| D-Duck | 1.591 [1.535–1.638], n=5 | 2.707 | 159.647 | unmeasured | unmeasured | 2.093 [1.993–2.151], n=5 |

All ten retained matched EEVEE still comparisons favor final8 in this run (1.15–1.90×); no slower EEVEE case was dropped. Nineteen rows, including C-Sponza, lack a retained matched EEVEE result and cannot support a ratio. Some new still medians are slower than the prior 32-sample run (for example D-empty, B-street and MorphStressTest); lower samples do not guarantee lower full-command wall time. C-Sponza retains its upstream licence hold; no new render or ratio. Final uses 8 + OIDN; retained EEVEE stills use 64 and no denoising. These are different workflows, not quality parity.

## Birds — std/draft 4 + OIDN, defaults unmodified

| Shot | Full movie warm s (measured) | First full movie s | 320×180 warmup s | Setup s | EEVEE 16-sample s (projected, both retained runs) |
|---|---|---|---|---|---|
| skim | 96.942 [95.428–96.999], n=3 | 95.428 | 166.030 | 243.809 | 1511.108 / 1817.625 |
| dive | 124.718 [120.437–126.827], n=3 | 120.437 | 112.994 | 167.376 | 2517.891 / 2417.831 |
| reveal | 153.411 [148.839–154.513], n=3 | 148.839 | 113.910 | 163.218 | 2916.237 / 2889.758 |

Three complete movies per shot. Skim 192 frames, dive 240, reveal 288, at 24 fps and 1080p. The bird protocol warms a separate low-resolution still after setup; first full movies are shown separately but are included in the three-movie median. Both retained Blender projection runs are shown; no denominator is selected for advantage. The EEVEE projections use 16 samples against vork's four, so they are not like for like. The four-sample EEVEE projection exists only for skim: 773.7–1,082.5 s across the retained runs, with ray tracing on or off, about 8–11× the 96.9 s movie. Dive and reveal have none.

## Carried-forward std/previs

| Cell | Warm s (prior freeze measured) | EEVEE comparison where available |
|---|---|---|
| bird-skim-previs | 48.990 [46.473–49.020], n=3 | unmeasured |
| bird-birds2-previs | 50.127 [50.093–50.204], n=3 | unmeasured |
| preview7-D-cube-still | 1.284 [1.234–1.286], n=5 | see previous report |
| preview7-D-cube-video | 3.062 [2.949–3.321], n=3 | unmeasured |
| preview7-B-blocking-still | 1.030 [1.029–1.078], n=5 | see previous report |
| preview7-B-blocking-video | 3.496 [3.495–3.498], n=3 | unmeasured |
| preview7-B-interior-still | 1.337 [1.234–1.433], n=5 | see previous report |
| preview7-B-interior-video | 4.565 [4.507–4.659], n=3 | unmeasured |
| preview7-B-car-pan-still | 1.285 [1.234–1.340], n=5 | see previous report |
| preview7-B-car-pan-video | 6.391 [6.187–6.584], n=3 | unmeasured |
| preview7-B-night-still | 1.333 [1.282–1.389], n=5 | see previous report |
| preview7-B-night-video | 5.864 [5.808–5.931], n=3 | unmeasured |
| preview7-C-ToyCar-still | 1.542 [1.485–1.591], n=5 | see previous report |
| preview7-C-ToyCar-video | 4.107 [4.103–4.158], n=3 | unmeasured |
| preview7-C-SheenChair-still | 1.030 [1.029–1.032], n=5 | see previous report |
| preview7-C-SheenChair-video | 3.013 [2.947–3.043], n=3 | unmeasured |
| repo-B-crowd-previs-still | 1.132 [1.079–1.185], n=5 | see previous report |
| repo-B-crowd-previs-video | 4.097 [4.056–4.267], n=3 | unmeasured |
| repo-B-push-in-previs-still | 1.385 [1.337–1.434], n=5 | see previous report |
| repo-B-push-in-previs-video | 3.705 [3.700–3.711], n=3 | unmeasured |
| repo-B-rack-focus-previs-still | 1.285 [1.233–1.388], n=5 | see previous report |
| repo-B-rack-focus-previs-video | 3.902 [3.799–3.907], n=3 | unmeasured |
| repo-B-street-previs-still | 1.283 [1.231–1.391], n=5 | see previous report |
| repo-B-street-previs-video | 3.964 [3.953–4.061], n=3 | unmeasured |
| repo-B-turntable-previs-still | 1.334 [1.333–1.341], n=5 | see previous report |
| repo-B-turntable-previs-video | 4.612 [4.563–4.712], n=3 | unmeasured |
| repo-B-walking-previs-still | 1.346 [1.281–1.441], n=5 | see previous report |
| repo-B-walking-previs-video | 2.190 [2.190–2.238], n=3 | unmeasured |

## Carried-forward std/path

| Scene | Path s (prior freeze measured) | Cycles s (reused measured) | Comparison |
|---|---|---|---|
| B-blocking | 23.878 [23.572–24.036], n=5 | 5.258 | 4.54× slower |
| B-push-in | 22.251 [22.001–22.710], n=5 | 4.425 | 5.03× slower |
| B-turntable | 17.215 [17.154–17.457], n=5 | 5.555 | 3.10× slower |
| B-walking | 10.917 [10.516–11.373], n=5 | 3.108 | 3.51× slower |
| B-interior | 53.205 [52.855–53.624], n=5 | unmeasured | unmeasured |
| B-street | 38.007 [37.355–38.097], n=5 | unmeasured | unmeasured |
| B-rack-focus | 24.222 [23.723–24.240], n=5 | unmeasured | unmeasured |
| B-car-pan | 26.901 [26.469–27.347], n=5 | 4.632 | 5.81× slower |
| B-crowd | 37.296 [36.321–38.150], n=5 | unmeasured | unmeasured |
| B-night | 31.860 [31.616–31.875], n=5 | 16.041 | 1.99× slower |
| C-Sponza | unmeasured | unmeasured | unmeasured |
| C-ABeautifulGame | 19.670 [19.083–19.877], n=5 | unmeasured | unmeasured |
| C-SheenChair | 11.430 [11.278–12.093], n=5 | 2.962 | 3.86× slower |
| C-ToyCar | 18.029 [17.679–18.128], n=5 | 4.777 | 3.77× slower |
| C-Lantern | 9.460 [9.455–9.715], n=5 | unmeasured | unmeasured |
| C-ClearcoatWicker | 11.271 [10.567–12.110], n=5 | unmeasured | unmeasured |
| C-GlassVaseFlowers | 20.911 [20.892–21.124], n=5 | unmeasured | unmeasured |
| C-GlamVelvetSofa | 9.602 [9.101–9.651], n=5 | unmeasured | unmeasured |
| C-StainedGlassLamp | 47.721 [47.603–48.002], n=5 | unmeasured | unmeasured |
| C-IridescenceLamp | 13.817 [13.311–14.477], n=5 | unmeasured | unmeasured |
| C-AnisotropyBarnLamp | 14.637 [14.430–15.085], n=5 | unmeasured | unmeasured |
| C-CommercialRefrigerator | 29.737 [29.626–30.129], n=5 | unmeasured | unmeasured |
| C-CesiumMilkTruck | 10.516 [10.467–10.869], n=5 | unmeasured | unmeasured |
| C-RecursiveSkeletons | 11.373 [10.469–11.487], n=5 | unmeasured | unmeasured |
| C-MorphStressTest | 10.413 [10.162–11.473], n=5 | unmeasured | unmeasured |
| C-NodePerformanceTest | 47.134 [47.034–49.791], n=5 | unmeasured | unmeasured |
| D-empty | 10.868 [10.583–10.971], n=5 | unmeasured | unmeasured |
| D-cube | 9.253 [9.156–9.604], n=5 | 2.558 | 3.62× slower |
| D-Duck | 10.008 [9.349–10.715], n=5 | unmeasured | unmeasured |

Path timings originated in frozen-44ccb8d57 and were already carried forward by frozen-04a1a993a. They are not measured on this commit.
