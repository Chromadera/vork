COMPOSITOR
render.compositor: ordered material-name|{material,name?,resolution?} entries; last image is output; material may be inline.
resolution: [width,height] or positive output scale (floor, minimum 1); omitted = output; final image resampled to output.
Image bindings: {node:earlier-name}, {image:name}, {channel:color|depth|view_normal|object_index|motion|shadow_catcher}, {aov:name}; names are local to each graph.
<!-- all -->
COMPOSITOR PROGRAM REFERENCE
float4 composite(CompositeContext,Inputs): scene-linear premultiplied RGBA; compositor_sample(uv) reads previous result (first reads accumulated beauty).
compositor_sample(image,uv): clamped bilinear; compositor_load(image,int2): clamped pixel; compositor_size(image): pixels; compositor_size() reads previous extent. UV origin top left; ctx.size is working extent.
Runs after shutter accumulation/denoising, before exposure/display. No history. std/bloom stages: prepare/downsample/upsample/combine; post/glare returns additive RGBA for a separate merge.

std/bloom follows Blender bloom arithmetic through an explicit image pyramid; no stages or bindings are inserted automatically.
Inputs (defaults): stage=combine (prepare|downsample|upsample|combine); original, source, fine are Image bindings with no default.
Stage contract:
prepare: requires original; box-reduce original by divisor, then threshold highlights. Output at the chosen pyramid base resolution.
downsample: requires source from prepare or a finer downsample; output at half source resolution. karis enables peak-intensity-weighted reduction.
upsample: requires coarse source and its next finer pyramid image fine; output at fine's resolution, adding the filtered source to fine.
combine: requires original and source (the reconstructed pyramid, or prepare for one level); output at original resolution. Preserves original alpha.
Bind original explicitly to {channel:color} for accumulated beauty, or a retained earlier full-resolution node. It is not compositor_sample(uv)'s previous result.
Name each reused pyramid image; connect downsample nodes toward smaller images, then upsample back toward prepare; combine original and the final reconstructed source.
Pyramid depth and base resolution are authored. levels=1 is the number of image contributions summed into reconstructed source (prepare plus downsample levels); combine divides bloom strength by levels.
divisor=1: integer box-reduction factor in original pixels, used by prepare. divisor and levels are uint; zero behaves as one. karis=false: downsample only.
threshold=1: nonnegative scene-linear highlight cutoff; smoothness=0.1: knee width in [0,1], limited to threshold.
clamp=false, maximum=10: optional nonnegative highlight-magnitude ceiling in prepare.
strength=1, saturation=1, tint=[1,1,1]: nonnegative combine controls; strength=0 preserves the nonnegative original.
Missing stage-required Image bindings are scene errors, including zero-strength combine nodes.
