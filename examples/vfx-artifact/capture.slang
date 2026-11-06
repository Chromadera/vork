import vork;
struct Inputs {};
float4 composite(CompositeContext ctx,Inputs i) { return compositor_sample(ctx.uv); }
