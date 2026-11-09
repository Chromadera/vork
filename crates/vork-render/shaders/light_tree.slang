// Standalone source selection: no material, connection or path imports.
import vork_constants;
public struct LightTreeNode {
    public float4 lower_power;
    public float4 upper_infinite;
    public uint4 links;
    public uint4 source;
};
public struct LightTreePick {
    public uint kind;
    public uint identity;
    public uint leaf;
    public float probability;
};
// Log scores keep the common power scale from underflowing during distance
// division. Only the final sibling ratio is exponentiated, after subtracting
// the larger score. The builder publishes normal positive power guides or zero.
float light_tree_log_importance(LightTreeNode node, float3 point) {
    if (!(node.lower_power.w > 0.)) return -REG_FP32_MAX;
    float power = log2(node.lower_power.w);
    if (node.upper_infinite.w != 0.) return power;
    float3 center = .5 * node.lower_power.xyz + .5 * node.upper_infinite.xyz;
    float3 radius = .5 * node.upper_infinite.xyz - .5 * node.lower_power.xyz;
    float3 delta = point - center;
    float distance = max(REG_LIGHT_TREE_MIN_DISTANCE_SQUARED, max(dot(delta, delta), dot(radius, radius)));
    if (!isfinite(distance)) return -REG_FP32_MAX;
    return power - log2(distance);
}
float light_tree_branch(LightTreeNode a, LightTreeNode b, float3 point) {
    float x = light_tree_log_importance(a, point), y = light_tree_log_importance(b, point);
    float scale = max(x,y);
    float uniform = float(a.links.w) / (float(a.links.w) + float(b.links.w));
    if (scale == -REG_FP32_MAX) return uniform;
    x = exp2(x-scale); y = exp2(y-scale);
    return x / (x+y);
}
public float light_tree_probability(StructuredBuffer<LightTreeNode> nodes, uint leaf, float3 point) {
    float probability = 1.;
    uint child = leaf;
    while (nodes[child].links.z != 0xffffffffu) {
        uint parent = nodes[child].links.z;
        uint2 children = nodes[parent].links.xy;
        float p = light_tree_branch(nodes[children.x], nodes[children.y], point);
        probability *= child == children.x ? p : 1.-p;
        child = parent;
    }
    return REG_LIGHT_TREE_UNIFORM_MIX / float(nodes[0].links.w)
        + (1. - REG_LIGHT_TREE_UNIFORM_MIX) * probability;
}
// Independent uniform coordinates in [0,1): x chooses the source, y chooses
// the strategy. Do not split x to choose the strategy: that drops source bits.
// Empty tree returns probability zero, leaf MAX.
public LightTreePick light_tree_pick(StructuredBuffer<LightTreeNode> nodes, uint node_count, float3 point, float2 u) {
    LightTreePick result = {}; result.leaf = 0xffffffffu;
    if (node_count == 0u) return result;
    uint node = 0u; float probability = 1.;
    float choice = clamp(u.x,0.,(1. - REG_FP32_EPSILON * .5));
    bool uniform = u.y < REG_LIGHT_TREE_UNIFORM_MIX;
    uint rank = min(uint(choice * float(nodes[0].links.w)), nodes[0].links.w-1u);
    while (nodes[node].links.x != 0xffffffffu) {
        uint2 children = nodes[node].links.xy;
        float p = light_tree_branch(nodes[children.x], nodes[children.y], point);
        bool left;
        if (uniform) {
            uint count = nodes[children.x].links.w;
            left = rank < count;
            if (!left) rank -= count;
        } else {
            left = choice < p;
            choice = left ? choice/p : (choice-p)/(1.-p);
            choice = min(choice,(1. - REG_FP32_EPSILON * .5));
        }
        probability *= left ? p : 1.-p;
        node = left ? children.x : children.y;
    }
    result.kind=nodes[node].source.x; result.identity=nodes[node].source.y;
    result.leaf=node;
    result.probability=REG_LIGHT_TREE_UNIFORM_MIX / float(nodes[0].links.w)
        + (1. - REG_LIGHT_TREE_UNIFORM_MIX) * probability;
    return result;
}
