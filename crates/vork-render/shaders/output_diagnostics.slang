// Exposure-wide 64-bit counters, using portable 32-bit atomics on Vulkan and Metal.
public void check_nonfinite(RWStructuredBuffer<uint> counters,uint stage,float4 value) {
    if(!all(isfinite(value)) && counters[0]!=0u) {
        uint previous;InterlockedAdd(counters[stage],1u,previous);
        if(previous==0xffffffffu) {InterlockedAdd(counters[stage+1u],1u);}
    }
}
