// The path integrator's one sampler: Owen-scrambled Sobol (Burley, "Practical
// Hash-based Owen Scrambling", JCGT 2020), the construction of Cycles' Sobol-Burley
// pattern. src/path_sampler.rs holds the CPU reference and its tests.
//
// Stratified dimensions come in groups of four. A group is one 4D Sobol point whose
// sample index is shuffled per pixel and group (a nested uniform scramble of the
// index bits), then each coordinate is Owen-scrambled with its own independently
// hashed seed, so every point is uniform on [0,1)^4 over the scramble. The four
// coordinates of a group are jointly stratified; coordinates 0 and 1 form a
// (0,2)-sequence, the best pair, so every 2D domain takes them. Independent
// shuffles decorrelate groups and pixels. The exposure sample index is the sequence
// index and the shuffle permutes within power-of-two blocks, so every power-of-two
// prefix of a pixel's samples is a net: adaptive stopping keeps stratification.
//
// The layout is this table; consumers read it only through the accessors below.
//   group 0, camera: pixel position, lens (aperture) point.
//   Each vertex v (one per traced segment: surface, volume scattering or alpha/null
//   continuation) owns groups 1+5v .. 5+5v:
//   scatter:     direction pair (BSDF, fibre or phase) and component pick; mixture lobe.
//   light:       point on the light; light pick.
//   mesh:        triangle barycentrics; triangle pick.
//   environment: environment direction.
//   decisions:   stochastic opacity, subsurface entry, Russian roulette;
//                the first vertex's unused fourth coordinate samples wavelength.
// The table has no end: group g at any depth hashes its own shuffle and scrambles,
// so arbitrarily deep paths keep fully stratified groups (padding with independently
// scrambled Sobol rather than white noise).
// Consumers that draw a data-dependent number of values keep the hashed white-noise
// stream (`path_random` below): free-flight and delta tracking in media, random-walk
// subsurface, opacity and lobe choices along connection rays, light-sampling threshold
// kills and lit-stage shadow rays. Shutter time is not a per-pixel dimension: the host
// poses the scene once per exposure sample (sampling::shutter_time), and the per-pixel
// shuffle decorrelates path dimensions from it.
import vork_constants;
import vork_sampling;

static const uint PATH_SAMPLE_CAMERA=0u;
static const uint PATH_SAMPLE_FIRST_VERTEX=1u;
static const uint PATH_SAMPLE_SCATTER=0u;
static const uint PATH_SAMPLE_LIGHT=1u;
static const uint PATH_SAMPLE_MESH=2u;
static const uint PATH_SAMPLE_ENVIRONMENT=3u;
static const uint PATH_SAMPLE_DECISIONS=4u;
static const uint PATH_SAMPLE_GROUPS_PER_VERTEX=5u;

public struct PathSampler {
    // Exposure sample index, the pixel's hashed seed, the index mask of the
    // power-of-two block holding the exposure, and the current vertex.
    public uint index; public uint seed; public uint mask; public uint vertex;
};
// The power-of-two block holding `count` samples.
public uint sample_block(uint count) {return count<=1u ? 1u : 2u<<firstbithigh(count-1u);}
public PathSampler path_sampler(uint2 pixel,uint index,uint count,uint stream) {
    PathSampler s;s.index=index;s.seed=screen_ray_hash(pixel,0u,stream);s.vertex=0u;
    // Shuffled indices of the first 2^k samples stay a permutation of 0..2^k-1:
    // Sobol's first 2^k points, at most k generator columns per coordinate.
    s.mask=sample_block(count)-1u;
    return s;
}
// Laine-Karras style hash of a bit-reversed integer (Vegdahl's constants, as in
// Cycles): every output bit depends only on itself and lower input bits, so in
// the reversed domain it is an Owen (nested uniform) scramble. Its seed enters
// bit 0 only by addition, so seeds that differ by a constant would flip the same
// top bit: the seed is hashed here, and distinct seeds give independent scrambles.
public uint path_owen_reversed(uint x,uint seed) {
    seed=pixel_hash(seed);
    x^=x*0x3d20adeau;x+=seed;x*=(seed>>16u)|1u;x^=x*0x05526c56u;x^=x*0x53a22864u;return x;
}
// A 32-bit fraction to (0,1): centres of 2^23 buckets, exact in float.
public float sample_unit(uint x) {return (float(x>>9u)+.5)*(1./8388608.);}
// Hashed white noise for consumers that draw a data-dependent number of values;
// every fixed path dimension comes from the groups below.
public float path_random(inout uint state) {state=pixel_hash(state+REG_PIXEL_HASH_STREAM_STEP);return sample_unit(state);}
public float4 path_sample_group(PathSampler s,uint group) {
    let seed=pixel_hash(s.seed^pixel_hash(group+1u));
    // Five scrambles per group (index shuffle, four coordinates). Their seeds differ
    // by scattered constants: pixel_hash avalanches poorly into its low bits for
    // inputs that differ only in low bits (seed+1, seed+2 correlate the coordinates).
    let index=reversebits(path_owen_reversed(reversebits(s.index),seed^0x6c8e9cf5u))&s.mask;
    // Sobol dimensions 1-3 from their generator recurrences (Joe-Kuo primitive
    // polynomials of degree 1, 2, 3), one column per index bit, without a table.
    var x=uint3(0u);var p=0x80000000u;var q=uint2(0x80000000u,0xc0000000u);var r=uint3(0x80000000u,0xc0000000u,0x20000000u);
    for (var bits=index;bits!=0u;bits>>=1u) {
        if ((bits&1u)!=0u) {x^=uint3(p,q.x,r.x);}
        p^=p>>1u;
        q=uint2(q.y,q.y^q.x^(q.x>>2u));
        r=uint3(r.y,r.z,r.y^r.x^(r.x>>3u));
    }
    // Owen-scramble each coordinate in the bit-reversed domain. Dimension 0's
    // radical inverse, reversed, is the index itself.
    let scrambled=uint4(path_owen_reversed(index,seed^0xa511e9b3u),path_owen_reversed(reversebits(x.x),seed^0x63d83595u),
        path_owen_reversed(reversebits(x.y),seed^0x2f7c4a1bu),path_owen_reversed(reversebits(x.z),seed^REG_PIXEL_HASH_STREAM_STEP));
    let value=reversebits(scrambled);
    return float4(sample_unit(value.x),sample_unit(value.y),sample_unit(value.z),sample_unit(value.w));
}
float4 path_vertex_group(PathSampler s,uint slot) {
    return path_sample_group(s,PATH_SAMPLE_FIRST_VERTEX+s.vertex*PATH_SAMPLE_GROUPS_PER_VERTEX+slot);
}

// xy the position within the pixel, zw the unit-square lens point.
public float4 path_camera_sample(PathSampler s) {return path_sample_group(s,PATH_SAMPLE_CAMERA);}
// xy the direction pair and z the component pick, the convention transport_sample,
// strand_sample and phase_direction share; w the material mixture lobe.
public float4 path_scatter(PathSampler s) {return path_vertex_group(s,PATH_SAMPLE_SCATTER);}
// xy the conditional emitter coordinates; zw independent source/strategy picks.
public float4 path_light(PathSampler s) {return path_vertex_group(s,PATH_SAMPLE_LIGHT);}
// emitter_mesh_sample's order: x the triangle pick, yz its barycentrics.
public float3 path_triangle(PathSampler s) {return path_vertex_group(s,PATH_SAMPLE_MESH).zxy;}
public float2 path_environment(PathSampler s) {return path_vertex_group(s,PATH_SAMPLE_ENVIRONMENT).xy;}
public float path_opacity(PathSampler s) {return path_vertex_group(s,PATH_SAMPLE_DECISIONS).x;}
public float path_subsurface_entry(PathSampler s) {return path_vertex_group(s,PATH_SAMPLE_DECISIONS).y;}
public float path_roulette(PathSampler s) {return path_vertex_group(s,PATH_SAMPLE_DECISIONS).z;}

// A dedicated unused coordinate, stratified for every adaptive power-of-two prefix.
public float path_wavelength_sample(PathSampler s) {return path_sample_group(s,PATH_SAMPLE_FIRST_VERTEX+PATH_SAMPLE_DECISIONS).w;}

// Radiance eta^-2 is a change of measure, not lost survival energy. Both
// continuation providers compensate the current/camera IOR before roulette.
public float path_survival(float3 throughput,float relative_ior) {
    let energy=throughput*(relative_ior*relative_ior);
    return clamp(max(energy.x,max(energy.y,energy.z)),REG_PATH_ROULETTE_MIN,REG_PATH_ROULETTE_MAX);
}
