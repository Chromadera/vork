// std/openpbr threshold evaluation, also used by glTF MASK lowering.
public float std_opacity(float value, float cutoff) {
    return cutoff < 0. ? value : (value >= cutoff ? 1. : 0.);
}
