// Bradford chromatic adaptation, Blender source illuminant convention.
// Robertson (1968) isotherms independently sourced from Colour (BSD-3-Clause).
// See dist/licenses/colour-robertson.txt; no Blender renderer source is included.
import vork;
public struct Inputs {
    [InputRange(1800.0, 100000.0, true, true)] float temperature = 6500.;
    [InputRange(-500.0, 500.0, true, true)] float tint = 10.;
};
float3 illuminant(float temperature, float tint) {
    const float4 PLANCK_LOCUS[31] = {
        float4(0.0, 0.18006, 0.26352, -0.24341),
        float4(10.0, 0.18066, 0.26589, -0.25479),
        float4(20.0, 0.18133, 0.26846, -0.26876),
        float4(30.0, 0.18208, 0.27119, -0.28539),
        float4(40.0, 0.18293, 0.27407, -0.3047),
        float4(50.0, 0.18388, 0.27709, -0.32675),
        float4(60.0, 0.18494, 0.28021, -0.35156),
        float4(70.0, 0.18611, 0.28342, -0.37915),
        float4(80.0, 0.1874, 0.28668, -0.40955),
        float4(90.0, 0.1888, 0.28997, -0.44278),
        float4(100.0, 0.19032, 0.29326, -0.47888),
        float4(125.0, 0.19462, 0.30141, -0.58204),
        float4(150.0, 0.19962, 0.30921, -0.70471),
        float4(175.0, 0.20525, 0.31647, -0.84901),
        float4(200.0, 0.21142, 0.32312, -1.0182),
        float4(225.0, 0.21807, 0.32909, -1.2168),
        float4(250.0, 0.22511, 0.33439, -1.4512),
        float4(275.0, 0.23247, 0.33904, -1.7298),
        float4(300.0, 0.2401, 0.34308, -2.0637),
        float4(325.0, 0.24792, 0.34655, -2.4681),
        float4(350.0, 0.25591, 0.34951, -2.9641),
        float4(375.0, 0.264, 0.352, -3.5814),
        float4(400.0, 0.27218, 0.35407, -4.3633),
        float4(425.0, 0.28039, 0.35577, -5.3762),
        float4(450.0, 0.28863, 0.35714, -6.7262),
        float4(475.0, 0.29685, 0.35823, -8.5955),
        float4(500.0, 0.30505, 0.35907, -11.324),
        float4(525.0, 0.3132, 0.35968, -15.628),
        float4(550.0, 0.32129, 0.36011, -23.325),
        float4(575.0, 0.32931, 0.36038, -40.77),
        float4(600.0, 0.33724, 0.36051, -116.45),
    };
    float mired = 1e6 / temperature;
    int upper = 1;
    while (upper < 30 && PLANCK_LOCUS[upper].x < mired) upper++;
    float4 lo = PLANCK_LOCUS[upper - 1], hi = PLANCK_LOCUS[upper];
    float weight = (mired - lo.x) / (hi.x - lo.x);
    float2 direction = normalize(lerp(normalize(float2(1., lo.w)), normalize(float2(1., hi.w)), weight));
    float2 uv = lerp(lo.yz, hi.yz, weight) - direction * (tint / 3000.);
    return float3(1.5 * uv.x / uv.y, 1., (4. - uv.x - 10. * uv.y) / (2. * uv.y));
}
float3x3 inverse3(float3x3 m) {
    float3 a = cross(m[1], m[2]);
    float3 b = cross(m[2], m[0]);
    float3 c = cross(m[0], m[1]);
    return transpose(float3x3(a, b, c)) / dot(m[0], a);
}
float3 white_balance(float3 color, Inputs inputs) {
    // Matrices below are written in row order; mul(matrix, vector).
    const float3x3 RGB_TO_XYZ = float3x3(
        .4124564, .3575761, .1804375,
        .2126729, .7151522, .0721750,
        .0193339, .1191920, .9503041);
    const float3x3 XYZ_TO_LMS = float3x3(
        .8951, .2664, -.1614,
        -.7502, 1.7135, .0367,
        .0389, -.0685, 1.0296);
    float3 source = mul(XYZ_TO_LMS, illuminant(inputs.temperature, inputs.tint));
    float3 target = mul(XYZ_TO_LMS, float3(.95047, 1., 1.08883));
    // Build the same linear matrix as the former CPU display stage.
    float3 ratio = target / source;
    float3x3 scale = float3x3(ratio.x,0.,0., 0.,ratio.y,0., 0.,0.,ratio.z);
    float3x3 adaptation = mul(mul(mul(mul(inverse3(RGB_TO_XYZ), inverse3(XYZ_TO_LMS)), scale), XYZ_TO_LMS), RGB_TO_XYZ);
    return mul(adaptation, color);
}
public float4 composite(CompositeContext ctx, Inputs inputs) {
    float4 color = compositor_sample(ctx.uv);
    return float4(white_balance(color.rgb, inputs), color.a);
}
