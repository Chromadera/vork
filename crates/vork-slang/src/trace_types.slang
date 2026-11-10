public struct TraceHit {
    public bool hit; public float distance; public float3 position;
    public float3 geometric_normal; public float3 shading_normal;
    public uint object_id; public uint instance_index; public uint primitive_id;
    public float3 barycentrics; public float2 uv; public bool front_facing;
};
