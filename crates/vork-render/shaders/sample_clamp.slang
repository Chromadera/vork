import vork_constants;
// Path contribution clamp shared by every radiance accumulation site.
// Cycles film clamp: mean absolute RGB, with one scale to preserve hue.
// A non-finite contribution (an infinite light sample, or inf*0 = NaN) is
// dropped as zero: one bad sample must not make the pixel, and so the frame,
// non-finite. Finite contributions are unchanged.
public float3 clamp_sample(float3 value, float limit, inout uint dropped) {
    float brightness=dot(abs(value),float3(1./3.));
    if (!isfinite(brightness)) {
        dropped++;
        // Keep the final multiply outside the branch so finite contributions
        // can retain the accumulator's multiply-add contraction.
        value=float3(0.);
        brightness=0.;
    }
    return value*select(limit>0. && brightness>limit,limit/max(brightness,1e-20),1.);
}

// Flush one path's total only when it dropped a contribution. Keeping buffer
// atomics out of clamp_sample lets finite clamps remain ordinary arithmetic.
public void record_dropped_light_samples(RWStructuredBuffer<uint> diagnostics, uint dropped) {
    if(dropped==0u) {return;}
    uint previous;InterlockedAdd(diagnostics[REG_DROPPED_LIGHT_SAMPLES_WORD],dropped,previous);
    if(previous>0xffffffffu-dropped) {InterlockedAdd(diagnostics[REG_DROPPED_LIGHT_SAMPLES_WORD+1u],1u);}
}
