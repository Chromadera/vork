COMPOSITOR
render.compositor: ordered material-name|{material,name?,resolution?} entries; last image is output; material may be inline.
resolution: [width,height] or positive output scale (floor, minimum 1); omitted = output; final image resampled to output.
Image bindings: {node:earlier-name}, {image:name}, {channel:color|depth|view_normal|object_index|motion|shadow_catcher}, {aov:name}; names are local to each graph.
Depth, view_normal and object_index describe the first visible hit, not hidden rear edges;
view_normal includes authored smoothing. The compositor has no temporal history.
Channel bindings return raw values, before file encoding: depth in .x is view-axis
metres (background 1e10); view_normal in .xyz is a signed unit vector in camera
space (background zero); object_index in .x is the numeric object ID (background
zero); motion in .xy is pixel displacement, +Y down, current minus previous.
<!-- all -->
COMPOSITOR PROGRAM REFERENCE
float4 composite(CompositeContext,Inputs): scene-linear premultiplied RGBA; compositor_sample(uv) reads previous result (first reads accumulated beauty).
File/video image samples become premultiplied RGBA before filtering.
compositor_sample(image,uv): clamped bilinear; compositor_load(image,int2): clamped pixel; compositor_size(image): pixels; compositor_size() reads previous extent. UV origin top left; ctx.size is working extent.
Runs after shutter accumulation/denoising, before exposure/display.
