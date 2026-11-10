import vork_shadow_coverage;
// The depth prepass entries: position-only opaque depth, single-sided winding and
// alpha-tested coverage. The forward pass tests depth Equal against all of them.
import vork_constants;
import frame;
import stream;
import point_stream;
import material;
import surface;
import variant;

[shader("vertex")]
Varying main_vs([[vk::location(0)]] VertexIn v, uint object : SV_VulkanInstanceID, [[vk::location(REG_CAMERA_INSTANCE_LOCATION)]] uint mapped : CAMERA_INSTANCE) {let id=camera_object(object,mapped);return vertex(point_vertex(v,id),id);}
[shader("vertex")]
float4 opaque_depth_vs([[vk::location(0)]] float3 p : POSITION, uint vertex_index : SV_VertexID, uint object : SV_VulkanInstanceID, [[vk::location(REG_CAMERA_INSTANCE_LOCATION)]] uint mapped : CAMERA_INSTANCE) : SV_Position {
    let id=camera_object(object,mapped);return opaque_depth(point_position(p,id,vertex_index),id);
}
[shader("vertex")]
OpaqueDepth opaque_single_depth_vs([[vk::location(0)]] float3 p : POSITION, uint vertex_index : SV_VertexID, uint object : SV_VulkanInstanceID, [[vk::location(REG_CAMERA_INSTANCE_LOCATION)]] uint mapped : CAMERA_INSTANCE) {
    let id=camera_object(object,mapped);return opaque_single_depth(point_position(p,id,vertex_index),id);
}
[shader("fragment")]
void opaque_single_depth_fs(OpaqueDepth v, bool front : SV_IsFrontFace) {
    if (front==(v.mirrored!=0u)) {discard;}
}
// Fibre capture vertices retain the material varying interface.
[shader("vertex")]
Varying shadow_vs([[vk::location(0)]] VertexIn v, uint object : SV_VulkanInstanceID) {return shadow_vertex(point_vertex(v,object),object);}
// Alpha-tested depth: cutout materials, then dithered ones, whose hashed
// coverage needs the primitive (forward_variant::FEATURE_DITHERED).
[shader("fragment")]
void depth_fs(Varying v, bool front : SV_IsFrontFace) {
    if (!masked_covers(v,front,uint(frame.temporal.x),uint(frame.temporal.y))) {discard;}
}
[shader("fragment")]
void hashed_depth_fs(Varying v, bool front : SV_IsFrontFace, uint primitive : SV_PrimitiveID) {
    v.primitive=primitive;
    if (!masked_covers(v,front,uint(frame.temporal.x),uint(frame.temporal.y))) {discard;}
}
