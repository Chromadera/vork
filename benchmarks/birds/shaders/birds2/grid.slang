struct Inputs {};
uint hash(uint x) { x ^= x >> 16; x *= 0x7feb352du; x ^= x >> 15; x *= 0x846ca68bu; return x ^ (x >> 16); }
void compute(ComputeContext c, Inputs i) {
    uint3 cell=uint3(clamp(floor((p_load(c.id.x)+16.)/1.),0.,31.));
    uint base=((cell.z*32+cell.y)*32+cell.x)*4;
    // Sorted four smallest (hashed priority, stable id) keys per cell.
    // Each insertion is an integer compare/exchange network; arrival order cannot affect it.
    uint key=(hash(c.id.x+c.seed)&0xfff00000u)|c.id.x;
    for(uint j=0;j<4 && key!=0xffffffffu;j++) {
        uint old=grid_atomic_min(base+j,key); key=max(key,old);
    }
}
