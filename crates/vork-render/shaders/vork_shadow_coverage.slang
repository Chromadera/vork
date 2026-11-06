// Scalar material coverage is the coverage component of RGB light visibility.
// Retained maps sample a spatially stable mask; triangle connections evaluate
// scalar attenuation and thin-sheet RGB transmission in ray_coverage.slang.
import vork_constants;
import vork_material_table;
import vork_sampling;
public float candidate_scalar_opacity(uint r, float alpha) {
    alpha=clamp(alpha,0.,1.);
    if (material_records[r].extra.y==0.) {return 1.;}
    return alpha;
}
public float shadow_opacity(uint r, MaterialUv m, float vertex_alpha) {
    return candidate_scalar_opacity(r,material_alpha(r,m,vertex_alpha));
}
public bool coverage_accepts(float opacity, uint2 seed, uint sample_index, uint sample_count, uint stream=REG_SAMPLING_STREAM_SHADOW_ALPHA) {
    if (opacity>=1.) {return true;}
    if (opacity<=0.) {return false;}
    // Camera layers need independent Bernoulli decisions across sample indices.
    // Light/receiver quadrature retains its existing finite-budget strata.
    let threshold=stream==REG_SAMPLING_STREAM_RASTER_ALPHA
        ? float(screen_ray_hash(seed,sample_index,stream)>>8u)*(256.*REG_RADICAL_INVERSE_SCALE)
        : sample_2d(seed,sample_index,sample_count,stream).x;
    return threshold<opacity;
}
