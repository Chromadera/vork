import vork_constants;
import vork_prelude;
struct Dispatch {uint4 blocks; uint4 draws; float4x4 selection_vp;};
[[vk::binding(0,0)]] ConstantBuffer<Frame> camera : register(b0);
[[vk::binding(1,0)]] StructuredBuffer<Object> objects : register(t1);
[[vk::binding(2,0)]] StructuredBuffer<uint> words : register(t2);
[[vk::binding(3,0)]] StructuredBuffer<uint4> records : register(t3);
[[vk::binding(4,0)]] RWStructuredBuffer<uint> scratch : register(u4);
[[vk::binding(5,0)]] RWStructuredBuffer<uint> commands : register(u5);
[[vk::binding(6,0)]] RWStructuredBuffer<uint> camera_instances : register(u6);
[[vk::binding(7,0)]] StructuredBuffer<uint> live_commands : register(t7);
[[vk::binding(8,0)]] ConstantBuffer<Dispatch> dispatch : register(b8);
// Bounds enclose every installed vertex. The interval radius deliberately uses
// absolute transforms twice, so cancellation cannot shrink the enclosure.
bool outside(Object object) {
    if ((object.metadata.y&REG_RAY_CAMERA)==0u) {return true;}
    let base=object.reserved+6u;
    if (words[base+6u]==0u) {return false;}
    let lo=float3(asfloat(words[base]),asfloat(words[base+1u]),asfloat(words[base+2u]));
    let hi=float3(asfloat(words[base+3u]),asfloat(words[base+4u]),asfloat(words[base+5u]));
    let center=(lo+hi)*.5;
    let extent=(hi-lo)*.5+REG_CAMERA_CULL_ERROR*max(abs(lo),abs(hi));
    let model=transpose(float4x4(object.model[0],object.model[1],object.model[2],float4(0.,0.,0.,1.)));
    let world=mul(float4(center,1.),model);
    let radius=mul(float4(extent,0.),abs(model));
    let vp=dispatch.draws.w!=0u ? dispatch.selection_vp : camera.vp;
    let clip=mul(world,vp);
    let spread=mul(radius,abs(vp));
    let magnitude=mul(mul(float4(abs(center)+extent,1.),abs(model)),abs(vp));
    let margin=spread+REG_CAMERA_CULL_ERROR*magnitude;
    if (!all(isfinite(clip)) || !all(isfinite(margin))) {return false;}
    return clip.x+clip.w < -margin.x-margin.w || clip.w-clip.x < -margin.x-margin.w
        || clip.y+clip.w < -margin.y-margin.w || clip.w-clip.y < -margin.y-margin.w
        || clip.z < -margin.z || clip.w-clip.z < -margin.w-margin.z;
}

groupshared uint scan[REG_CAMERA_CULL_GROUP];
// Inclusive prefix and block total, with all shared reads finished before reuse.
uint2 prefix(uint value,uint lane) {
    scan[lane]=value;GroupMemoryBarrierWithGroupSync();
    for(uint step=1u;step<REG_CAMERA_CULL_GROUP;step*=2u) {
        let add=lane>=step ? scan[max(lane,step)-step] : 0u;
        GroupMemoryBarrierWithGroupSync();scan[lane]+=add;GroupMemoryBarrierWithGroupSync();
    }
    let result=uint2(scan[lane],scan[REG_CAMERA_CULL_GROUP-1u]);
    GroupMemoryBarrierWithGroupSync();return result;
}
[shader("compute")]
[numthreads(REG_CAMERA_CULL_GROUP,1,1)]
void classify(uint3 group:SV_GroupID,uint lane:SV_GroupIndex) {
    let block=group.y*dispatch.blocks.y+group.x;
    if(block>=dispatch.blocks.x) {return;}
    let record=records[block*REG_CAMERA_CULL_BLOCK_VEC4S];let draw=records[dispatch.blocks.w+record.z*REG_CAMERA_CULL_DRAW_VEC4S+1u];
    let object=record.x+lane;
    let mask=records[block*REG_CAMERA_CULL_BLOCK_VEC4S+1u+lane/128u][(lane/32u)%4u];
    var visible=false;
    if(lane<record.y && (mask&(1u<<(lane%32u)))!=0u) {
        let first=records[dispatch.blocks.w+record.z*REG_CAMERA_CULL_DRAW_VEC4S].z;
        let count=draw.y!=0u ? min(draw.x,live_commands[record.z*5u+1u]) : draw.x;
        if(object-first<count) {visible=!outside(objects[object]);}
    }
    let sum=prefix(visible?1u:0u,lane);
    if(lane<record.y) {scratch[object]=sum.x|(visible?REG_CAMERA_VISIBLE_BIT:0u);}
    if(lane==0u) {scratch[dispatch.blocks.z+block*2u]=sum.y;}
}
[shader("compute")]
[numthreads(REG_CAMERA_CULL_GROUP,1,1)]
void offsets(uint3 group:SV_GroupID,uint lane:SV_GroupIndex) {
    let draw=group.y*dispatch.draws.y+group.x;if(draw>=dispatch.draws.x) {return;}
    // CPU-owned and excluded ranges retain their uploaded counts.
    if(records[dispatch.blocks.w+draw*REG_CAMERA_CULL_DRAW_VEC4S+1u].z!=0u) {return;}
    let record=records[dispatch.blocks.w+draw*REG_CAMERA_CULL_DRAW_VEC4S];var total=0u;
    for(uint first=0u;first<record.y;first+=REG_CAMERA_CULL_GROUP) {
        let block=record.x+first+lane;let active=first+lane<record.y;
        let value=active?scratch[dispatch.blocks.z+block*2u]:0u;
        let sum=prefix(value,lane);
        if(active) {scratch[dispatch.blocks.z+block*2u+1u]=total+sum.x-value;}
        total+=sum.y;
    }
    if(lane==0u) {commands[draw*5u+1u]=total;}
}
[shader("compute")]
[numthreads(REG_CAMERA_CULL_GROUP,1,1)]
void compact(uint3 group:SV_GroupID,uint lane:SV_GroupIndex) {
    let block=group.y*dispatch.blocks.y+group.x;if(block>=dispatch.blocks.x) {return;}
    let record=records[block*REG_CAMERA_CULL_BLOCK_VEC4S];if(lane>=record.y) {return;}
    let object=record.x+lane;let value=scratch[object];
    if((value&REG_CAMERA_VISIBLE_BIT)==0u) {return;}
    let first=records[dispatch.blocks.w+record.z*REG_CAMERA_CULL_DRAW_VEC4S].z;
    let base=scratch[dispatch.blocks.z+block*2u+1u];
    camera_instances[first+base+(value&~REG_CAMERA_VISIBLE_BIT)-1u]=object+1u;
}
