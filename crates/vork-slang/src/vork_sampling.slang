import vork_openpbr_constants;
// Shared integer avalanche for display dither, filtered DOF and screen rays.
public uint pixel_hash(uint seed) {
    var value=(seed^(seed>>16u))*REG_SAMPLE_HASH_MIX;
    value=(value^(value>>16u))*REG_SAMPLE_HASH_MIX;
    return value^(value>>16u);
}
public uint screen_sample_hash(uint sample_index, uint stream) {
    return pixel_hash(sample_index*REG_PIXEL_HASH_STREAM_STEP+stream);
}
// Avalanche the pixel before combining sample/stream, avoiding x/sample symmetry.
public uint screen_ray_hash(uint2 pixel, uint sample_index, uint stream) {
    let spatial=pixel_hash((pixel.x*REG_SAMPLE_HASH_MIX)^pixel.y);
    return pixel_hash(spatial^screen_sample_hash(sample_index,stream));
}
// Shared finite-exposure sequence. Every coordinate visits each of count strata
// once, for arbitrary (including prime and odd) budgets. Streams change the
// pixel rotation and index order, never the integrated domain.
public uint sample_gcd(uint a,uint b) {
    while (b!=0u) {let remainder=a%b;a=b;b=remainder;} return a;
}
// The visibility lattice's coprime step for `count` samples.
public uint sample_lattice_step(uint count) {return sample_coprime(count,uint(round(float(count)*REG_SAMPLE_LATTICE_RATIO)));}
public uint sample_coprime(uint count,uint start) {
    var step=max(start,1u);while (sample_gcd(step,count)!=1u) {step+=1u;}return step;
}
// Residues must be formed before narrowing to uint. Exposure-wide aperture
// budgets can have products larger than 2^32 even though every index fits uint.
uint sample_add_mod(uint a,uint b,uint count) {
    return a>=count-b ? a-(count-b) : a+b;
}
uint sample_mul_mod(uint a,uint b,uint count) {
    a%=count;var result=0u;
    while (b!=0u) {
        if ((b&1u)!=0u) {result=sample_add_mod(result,a,count);}
        b>>=1u;
        if (b!=0u) {a=sample_add_mod(a,a,count);}
    }
    return result;
}
public float2 sample_lattice(uint index,uint count) {
    count=max(count,1u);
    let step=sample_lattice_step(count);
    index%=count;
    return (float2(float(index),float(sample_mul_mod(index,step,count)))+.5)/float(count);
}
public float2 sample_2d(uint2 pixel,uint index,uint count,uint stream) {
    count=max(count,1u);
    let seed=screen_ray_hash(pixel,0u,stream);
    // An affine bijection decorrelates camera jitter and effect streams without
    // losing any radial/angular stratum. Use integer bits, not float(uint), to
    // keep rotations strictly below one on both host and shader.
    let step=sample_coprime(count,1u+(seed>>16u)%count);
    let order=sample_add_mod(sample_mul_mod(index%count,step,count),(seed&65535u)%count,count);
    let rotation=float2(float(seed&65535u),float(pixel_hash(seed)&65535u))/65536.;
    return frac(sample_lattice(order,count)+rotation);
}
// A contiguous part of the shared sequence. Setup is paid once per integral;
// advancing the two integer residues preserves every point without multiplying
// or searching for coprimes in the tap loop.
public struct Sample2DSequence {
    uint count;
    uint2 residue;
    uint2 stride;
    float2 rotation;
}
public Sample2DSequence sample_2d_sequence(uint2 pixel,uint index,uint count,uint stream) {
    count=max(count,1u);
    let seed=screen_ray_hash(pixel,0u,stream);
    let step=sample_coprime(count,1u+(seed>>16u)%count);
    let lattice=sample_lattice_step(count);
    let order=sample_add_mod(sample_mul_mod(index%count,step,count),(seed&65535u)%count,count);
    let stride=step%count;
    let rotation=float2(float(seed&65535u),float(pixel_hash(seed)&65535u))/65536.;
    Sample2DSequence sequence;
    sequence.count=count;
    sequence.residue=uint2(order,sample_mul_mod(order,lattice,count));
    sequence.stride=uint2(stride,sample_mul_mod(stride,lattice,count));
    sequence.rotation=rotation;
    return sequence;
}
public float2 sample_2d_next(inout Sample2DSequence sequence) {
    let point=(float2(float(sequence.residue.x),float(sequence.residue.y))+.5)/float(sequence.count);
    sequence.residue=uint2(sample_add_mod(sequence.residue.x,sequence.stride.x,sequence.count),
        sample_add_mod(sequence.residue.y,sequence.stride.y,sequence.count));
    return frac(point+sequence.rotation);
}
// Validated camera budgets keep index*step within uint.
public float2 sample_lattice_precomputed(uint index,uint count,uint step) {
    return (float2(float(index),float((index*step)%count))+.5)/float(count);
}
// The host supplies precisely the same coprime parameters as sample_2d.
// Requires (count-1)*max(permutation,lattice)+(count-1) to fit uint.
// Shadow-ray samples satisfy this for their complete validated domain: at most
// 4096 camera samples * 4 rays. Wider exposure-wide filters use sample_2d.
public float2 sample_2d_precomputed(uint seed,uint index,uint count,uint permutation,uint lattice) {
    let order=((index%count)*permutation+(seed&65535u)%count)%count;
    let point=sample_lattice_precomputed(order,count,lattice);
    let rotation=float2(float(seed&65535u),float(pixel_hash(seed)&65535u))/65536.;
    return frac(point+rotation);
}
// Camera samples are validated at <=4096: both affine and lattice products fit uint.
public float2 sample_2d_installed(StructuredBuffer<uint> parameters,uint base,uint2 pixel,uint index,uint count,uint stream) {
    count=max(count,1u);
    let seed=screen_ray_hash(pixel,0u,stream);
    return sample_2d_precomputed(seed,index,count,parameters[base+2u+(seed>>16u)%count],parameters[base+1u]);
}
// Exact modular initialization also supports wider filter domains. Only setup
// multiplies; every subsequent tap advances residues with overflow-safe addition.
public Sample2DSequence sample_2d_sequence_installed(StructuredBuffer<uint> parameters,uint base,uint2 pixel,uint index,uint count,uint stream) {
    count=max(count,1u);
    let seed=screen_ray_hash(pixel,0u,stream);
    let step=parameters[base+2u+(seed>>16u)%count];
    let lattice=parameters[base+1u];
    let order=sample_add_mod(sample_mul_mod(index%count,step,count),(seed&65535u)%count,count);
    Sample2DSequence sequence;
    sequence.count=count;
    sequence.residue=uint2(order,sample_mul_mod(order,lattice,count));
    sequence.stride=uint2(step%count,sample_mul_mod(step%count,lattice,count));
    sequence.rotation=float2(float(seed&65535u),float(pixel_hash(seed)&65535u))/65536.;
    return sequence;
}

public float2 polar_disk(float2 u) {let angle=u.y*REG_TAU;return sqrt(u.x)*float2(cos(angle),sin(angle));}

// Shape a unit-disk aperture point (camera samples and filtered aperture taps).
public float2 aperture_point(float2 disk, float4 shape) {
    var point=disk;
    if (shape.x>=3.) {
        let sector=REG_TAU/shape.x;
        let phi=atan2(disk.y,disk.x);
        let coordinate=(phi-REG_TAU*floor(phi/REG_TAU))/sector;
        let angle=floor(coordinate)*sector+shape.y;
        point=lerp(float2(cos(angle),sin(angle)),float2(cos(angle+sector),sin(angle+sector)),frac(coordinate))*length(disk);
    }
    return point*float2(1./max(shape.z,1.),min(shape.z,1.));
}
