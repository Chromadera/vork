// Decoupled shading: merge one batch of visibility passes' keys into the
// per-pixel slots (decoupled.slang). The slots buffer holds, in order:
// REG_VISIBILITY_SLOTS dense slots per pixel, one overflow list head per pixel,
// then the overflow pool. A dense slot is a key (object + 1, primitive; word 0
// is zero while free) and the mask of samples it covers; a pool entry is
// the next entry + 1 (0 ends the list), then a key and mask. Every distinct key
// keeps its own coverage: a pixel with more keys than dense slots lists the rest.
import vork_constants;
import decoupled_slots;

[[vk::binding(0,0)]] Texture2DArray<uint2> keys;
[[vk::binding(1,0)]] RWStructuredBuffer<uint> slots;
// The pool's allocations attempted this exposure, the most any exposure of the
// frame attempted once the pool was full (zero if it never was; frame.rs grows
// the pool to it), and the pool's entries.
[[vk::binding(2,0)]] RWStructuredBuffer<uint> overflow;
// The batch's transparent fragments (decoupled_visibility.slang transparent_fragments_fs);
// capacity zero when the scene has none.
[[vk::binding(3,0)]] StructuredBuffer<uint> fragments;
// The batch's samples first..first+count of samples, one key layer each; first is
// a multiple of the batch size, a divisor of 32, so they share one mask word.
// Metal numbers buffers in declaration order, matching the binding order.
struct MergePass { uint first; uint count; uint samples; };
[[vk::binding(4,0)]] ConstantBuffer<MergePass> merge_pass;
uint allocate(SlotLayout layout, uint head) {
    uint n;InterlockedAdd(overflow[0],1u,n);
    var entry=0u;
    if (n>=overflow[2]) {InterlockedMax(overflow[1],n+1u);}
    else {
        let at=layout.entry(n);slots[at]=head;
        for (uint m=0u;m<layout.masks;m++) {slots[at+1u+REG_VISIBILITY_KEY_WORDS+m]=0u;}
        entry=n+1u;
    }
    return entry;
}

// Adds `bits` (mask word `word`) to key k's slot, allocating one if it is new.
// Select first, then update once: returning from the nested search makes
// llvmpipe lower a descriptor load on an empty execution mask to a null pointer.
// Keep the dense search's trip count uniform, including lanes already matched.
void insert(uint2 k, uint bits, SlotLayout layout, uint pixel, uint word) {
    var found=false;var chosen=0u;
    for (uint s=0u;s<REG_VISIBILITY_SLOTS;s++) {
        if (!found) {
            let at=layout.dense(pixel,s);
            if (slots[at]==0u || (slots[at]==k.x && slots[at+1u]==k.y)) {chosen=at;found=true;}
        }
    }
    let head=layout.head(pixel);
    if (!found) {
        for (var e=slots[head];e!=0u;e=slots[layout.entry(e-1u)]) {
            let at=layout.entry(e-1u)+1u;
            if (slots[at]==k.x && slots[at+1u]==k.y) {chosen=at;found=true;break;}
        }
    }
    if (!found) {
        let e=allocate(layout,slots[head]);
        if (e!=0u) {slots[head]=e;chosen=layout.entry(e-1u)+1u;found=true;}
    }
    if (found) {slots[chosen]=k.x;slots[chosen+1u]=k.y;slots[chosen+word]|=bits;}
}

// Distinct keys a batch gathers in registers before touching the slots.
static const uint GATHERED=4u;

[shader("compute")]
[numthreads(REG_COMPUTE_TILE,REG_COMPUTE_TILE,1)]
void merge(uint3 gid : SV_DispatchThreadID) {
    uint w,h,layers;keys.GetDimensions(w,h,layers);
    if (gid.x>=w || gid.y>=h) {return;}
    let layout=SlotLayout(w*h,merge_pass.samples);
    let pixel=gid.y*w+gid.x;
    let word=REG_VISIBILITY_KEY_WORDS+merge_pass.first/32u;
    // Each distinct key of the batch gathers its coverage bits before one slot
    // update, in order of first appearance (the slots' order): an interior pixel
    // touches its slot once per batch, an edge pixel once per triangle.
    uint2 gathered[GATHERED]={};uint bits[GATHERED]={};var n=0u;
    for (uint j=0u;j<merge_pass.count;j++) {
        let k=keys.Load(int4(gid.xy,j,0));
        if (k.x==0u) {continue;}
        let bit=1u<<((merge_pass.first+j)%32u);
        var found=false;
        for (uint t=0u;t<n;t++) {if (all(gathered[t]==k)) {bits[t]|=bit;found=true;break;}}
        if (found) {continue;}
        if (n==GATHERED) {
            for (uint t=0u;t<n;t++) {insert(gathered[t],bits[t],layout,pixel,word);}
            n=0u;
        }
        gathered[n]=k;bits[n]=bit;n++;
    }
    for (uint t=0u;t<n;t++) {insert(gathered[t],bits[t],layout,pixel,word);}
    // Then the transparent fragments the batch's passes listed for the pixel.
    let list=FragmentLayout(fragments[2],fragments[3],fragments[4]);
    if (list.capacity==0u) {return;}
    for (uint layer=0u;layer<merge_pass.count;layer++) {
        for (var e=fragments[list.head(layer,gid.xy)];e!=0u;e=fragments[list.record(e-1u)]) {
            let at=list.record(e-1u);
            insert(uint2(fragments[at+1u],fragments[at+2u]),fragments[at+3u],layout,pixel,word);
        }
    }
}
