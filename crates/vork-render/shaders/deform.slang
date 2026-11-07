// Packed vertex skinning and morphs, with current and previous pose rows.
import vork_constants;

struct Influence {
    uint4 JOINTS_0;
    float4 WEIGHTS_0;
};
[[vk::binding(0, 0)]] StructuredBuffer<float4> source : register(t0);
[[vk::binding(1, 0)]] StructuredBuffer<Influence> influences : register(t1);
[[vk::binding(2, 0)]] StructuredBuffer<float> morphs : register(t2);
[[vk::binding(3, 0)]] StructuredBuffer<float4> params : register(t3);
[[vk::binding(4, 0)]] RWStructuredBuffer<float> output : register(u4);
[[vk::binding(5, 0)]] StructuredBuffer<float4> previous_params : register(t5);
[[vk::binding(6, 0)]] RWStructuredBuffer<float> previous_output : register(u6);

float4 parameter(uint i, uint pose) {
    if (pose!=0u) {
        let header=previous_params[0];
        let stride=(REG_DEFORM_HEADER_FLOATS+uint(header.z)*REG_MATRIX_FLOATS+asuint(header.w)+REG_VEC4_FLOATS-1u)/REG_VEC4_FLOATS;
        return previous_params[i+(pose-1u)*stride];
    }
    return params[i];
}
// Store WGSL's columns as Slang rows, as the shared stream module does.
float4x4 matrix(uint j, uint pose) {
    uint b = REG_DEFORM_HEADER_FLOATS / REG_VEC4_FLOATS + j * REG_MATRIX_VECTORS;
    return float4x4(parameter(b, pose), parameter(b+1, pose),
        parameter(b+2, pose), parameter(b+3, pose));
}
float param(uint i, uint pose) {
    return parameter(i / REG_VEC4_FLOATS, pose)[i % REG_VEC4_FLOATS];
}

void store(uint at,uint pose,float value) {if(pose==0u) output[at]=value;else previous_output[at]=value;}

struct PoseVertex { float3 position; float3 normal; float4 tangent; };
PoseVertex evaluate_vertex(uint i, uint pose, float4 header) {
    uint count=asuint(header.x);
    uint sets=asuint(header.y);
    uint targets=asuint(header.w);
    uint src = i * REG_VERTEX_VECTORS;
    float4 pn = source[src];
    float4 nuv = source[src+1];
    float4 bind_tangent = source[src+REG_VERTEX_VECTORS-1];
    float3 p = pn.xyz;
    float3 n = float3(pn.w, nuv.xy);
    float3 tangent = bind_tangent.xyz;
    for (uint t = 0u; t < targets; t++) {
        float weight = param(REG_DEFORM_HEADER_FLOATS + uint(header.z) * REG_MATRIX_FLOATS + t, pose);
        if (weight == 0.) { continue; }
        uint delta = (t * count + i) * REG_MORPH_FLOATS;
        p += weight * float3(morphs[delta], morphs[delta+1], morphs[delta+2]);
        if (true) {
            n += weight * float3(morphs[delta+3], morphs[delta+4], morphs[delta+5]);
            tangent += weight * float3(morphs[delta+6], morphs[delta+7], morphs[delta+8]);
        }
    }
    float4x4 m = (float4x4)0;
    for (uint influence_set = 0u; influence_set < sets; influence_set++) {
        Influence inf = influences[i * sets + influence_set];
        m += matrix(inf.JOINTS_0.x, pose) * inf.WEIGHTS_0.x
           + matrix(inf.JOINTS_0.y, pose) * inf.WEIGHTS_0.y
           + matrix(inf.JOINTS_0.z, pose) * inf.WEIGHTS_0.z
           + matrix(inf.JOINTS_0.w, pose) * inf.WEIGHTS_0.w;
    }
    p = mul(float4(p, 1.), m).xyz;
    float3x3 linear = float3x3(m[0].xyz, m[1].xyz, m[2].xyz);
    float3x3 cofactor = float3x3(cross(linear[1], linear[2]),
        cross(linear[2], linear[0]), cross(linear[0], linear[1]));
    let determinant = dot(linear[0], cross(linear[1], linear[2]));
    // A collapsed skin still feeds displacement preparation while hidden.
    // Its normal has no inverse frame; retain the finite authored/morphed normal.
    if (determinant!=0.) {
        let transformed=mul(n,cofactor)*sign(determinant);
        let scale=max(abs(transformed.x),max(abs(transformed.y),abs(transformed.z)));
        // Zero morph/leaf-tip normals remain finite until the surface's face fallback.
        n=all(isfinite(transformed)) && scale>0. ? normalize(transformed/scale) : float3(0.);
    }
    tangent = mul(tangent, linear);
    PoseVertex result;
    result.position=p; result.normal=n; result.tangent=float4(tangent,bind_tangent.w * sign(determinant));
    return result;
}

[shader("compute")]
[numthreads(REG_DEFORM_GROUP, 1, 1)]
void evaluate(uint3 id : SV_DispatchThreadID) {
    uint i=id.x, pose=id.y;
    float4 header=parameter(0u,pose);
    if (i>=asuint(header.x)) {return;}
    PoseVertex vertex=evaluate_vertex(i,pose,header);
    // Split omitted-normal triangles are consecutive corners. Evaluate the
    // whole face at this pose before any NORMAL consumer (including displacement).
    // Reading immutable inputs avoids an inter-workgroup output dependency.
    if (source[i*REG_VERTEX_VECTORS+2].z!=0.) {
        uint face=(i/3u)*3u;
        float3 a=i==face ? vertex.position : evaluate_vertex(face,pose,header).position;
        float3 b=i==face+1u ? vertex.position : evaluate_vertex(face+1u,pose,header).position;
        float3 c=i==face+2u ? vertex.position : evaluate_vertex(face+2u,pose,header).position;
        float3 n=cross(b-a,c-a);
        vertex.normal=dot(n,n)>0. ? normalize(n) : float3(0.);
    }
    uint b=i*REG_DEFORM_VERTEX_FLOATS;
    if(pose!=0u) {b=i*REG_DEFORM_VERTEX_FLOATS*2u+(pose-1u)*REG_DEFORM_VERTEX_FLOATS;}
    store(b,pose,vertex.position.x);store(b+1,pose,vertex.position.y);store(b+2,pose,vertex.position.z);
    store(b+3,pose,vertex.normal.x);store(b+4,pose,vertex.normal.y);store(b+5,pose,vertex.normal.z);
    store(b+6,pose,vertex.tangent.x);store(b+7,pose,vertex.tangent.y);store(b+8,pose,vertex.tangent.z);store(b+9,pose,vertex.tangent.w);
}
