// Straight light-segment contract. Traversal consumes a sampled source;
// it never constructs another emitter. RGB transmittance includes coverage,
// null sheets and medium intervals. Final approximates solid transmission; path
// connections reserve solid boundaries for refracted transport.
public struct LightConnection {
    public uint identity;
    public uint sample_index;
    public bool delta;
    public float3 direction;
    public float3 endpoint;
    public float3 radiance;
    public float pdf;
    public float distance;
};
public struct Receiver {
    public float3 world;
    public float3 normal;
    public float3 position_error;
    public uint kind;
    public uint sample_index;
    public uint sample_count;
    public uint represented_mass;
};
public struct ConnectionPolicy {
    public uint groups;
    public bool all_casters;
    public uint ray_mask;
    public bool proven_opaque;
};
public struct ConnectionVisibility {
    public float3 transmittance;
    public uint transparent_crossings;
};
