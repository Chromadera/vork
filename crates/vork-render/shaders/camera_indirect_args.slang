// Size bounded connection dispatches from the two GPU-written request counts.
import vork_constants;
[[vk::binding(0,0)]] StructuredBuffer<uint> allocator : register(t0);
[[vk::binding(1,0)]] RWStructuredBuffer<uint> arguments : register(u1);
// Centroid launch capacity, sampled launch capacity, centroid blocks, sampled block width.
[[vk::binding(2,0)]] ConstantBuffer<uint4> config : register(b2);
[[vk::binding(3,0)]] RWStructuredBuffer<uint> errors : register(u3);
[shader("compute")][numthreads(REG_CAMERA_INDIRECT_GROUP,1,1)]
void prepare(uint3 id : SV_DispatchThreadID) {
    let sampled_blocks=config.y/config.w+uint(config.y%config.w!=0u);
    if(id.x>=config.z+sampled_blocks) {return;}
    let sampled=id.x>=config.z;
    let block=sampled ? config.w : REG_CAMERA_INDIRECT_BLOCK;
    let first=(sampled ? id.x-config.z : id.x)*block;
    let centroids=allocator[2];let samples=allocator[4];let capacity=allocator[1];
    let storage_short=centroids>capacity || samples>capacity-min(centroids,capacity);
    let launch_short=centroids>config.x || samples>config.y;
    // A true storage shortage still follows the frame's reservation growth.
    // Missing centroid launches report their own demand; sampled launches cover
    // the result reservation and cannot independently run short.
    if(id.x==0u && (storage_short || launch_short)) {
        let demand=storage_short ? centroids+min(samples,0xffffffffu-centroids) : max(centroids,samples);
        InterlockedMax(errors[3],demand);
    }
    let count=storage_short || launch_short ? 0u : (sampled ? samples : centroids);
    let records=count>first ? min(count-first,block) : 0u;
    arguments[id.x*3u]=(records+REG_CAMERA_INDIRECT_GROUP-1u)/REG_CAMERA_INDIRECT_GROUP;
    arguments[id.x*3u+1u]=1u;arguments[id.x*3u+2u]=1u;
}
