import vork;

// Blender Texture Coordinate outputs. Offsets are in raster pixels and use the
// kernel's local footprint, so a BumpHeight callback can reevaluate its complete
// expression at the finite-difference offsets passed by blender.vector.bump.
public struct TextureCoordinate {
    public float3 Generated;
    public float3 Normal;
    public float3 UV;
    public float3 Object;
    public float3 Camera;
    public float3 Window;
    public float3 Reflection;
};
// Evaluate at a footprint neighbour; scene transforms are shared with the
// renderer, including undeformed bounds and the current camera projection.
TextureCoordinate coordinates_at(ShadingContext ctx, float2 offset) {
    let position=ctx.POSITION+ctx.position_dx*offset.x+ctx.position_dy*offset.y;
    let rest=ctx.rest_position+ctx.rest_position_dx*offset.x+ctx.rest_position_dy*offset.y;
    let normal=ctx.unperturbed_normal+ctx.unperturbed_normal_dx*offset.x+ctx.unperturbed_normal_dy*offset.y;
    let incoming=ctx.incoming+ctx.incoming_dx*offset.x+ctx.incoming_dy*offset.y;
    TextureCoordinate result;
    result.Generated=object_to_generated(ctx,rest);
    result.Normal=world_normal_to_object(ctx,normal);
    result.UV=float3(ctx.TEXCOORD_0+ctx.duv_dx[0]*offset.x+ctx.duv_dy[0]*offset.y,0.);
    result.Object=world_to_object(ctx,position);
    result.Camera=world_to_camera(position);
    result.Window=world_to_window(position);
    result.Reflection=reflect(-normalize(incoming),normalize(normal));
    return result;
}
public TextureCoordinate texture_coordinate(ShadingContext ctx, float2 pixel_offset=float2(0.)) {
    let at=coordinates_at(ctx,float2(0.));
    let x=coordinates_at(ctx,float2(1.,0.));
    let y=coordinates_at(ctx,float2(0.,1.));
    TextureCoordinate result;
    result.Generated=at.Generated+(x.Generated-at.Generated)*pixel_offset.x+(y.Generated-at.Generated)*pixel_offset.y;
    result.Normal=normalize(at.Normal+(x.Normal-at.Normal)*pixel_offset.x+(y.Normal-at.Normal)*pixel_offset.y);
    result.UV=at.UV+(x.UV-at.UV)*pixel_offset.x+(y.UV-at.UV)*pixel_offset.y;
    result.Object=at.Object+(x.Object-at.Object)*pixel_offset.x+(y.Object-at.Object)*pixel_offset.y;
    result.Camera=at.Camera+(x.Camera-at.Camera)*pixel_offset.x+(y.Camera-at.Camera)*pixel_offset.y;
    result.Window=at.Window+(x.Window-at.Window)*pixel_offset.x+(y.Window-at.Window)*pixel_offset.y;
    result.Reflection=normalize(at.Reflection+(x.Reflection-at.Reflection)*pixel_offset.x+(y.Reflection-at.Reflection)*pixel_offset.y);
    return result;
}
