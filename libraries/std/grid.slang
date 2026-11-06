import vork;
#ifndef VORK_DENSE_GRIDS_ONLY
#define VORK_DENSE_GRIDS_ONLY 0
#endif
// Read-only NanoVDB accessor, pinned to OpenVDB 13.1 (Apache-2.0).
// Handles address the existing retained material/medium word buffer. A length
// precedes each grid so even a corrupt address cannot read another resource.
#define PNANOVDB_HLSL
#define VORK_FLOAT_GRIDS_ONLY
#define PNANOVDB_BUF_CUSTOM
#define pnanovdb_grid_type_t uint
#define PNANOVDB_GRID_TYPE_GET(grid_typeIn, nameIn) pnanovdb_grid_type_constants[grid_typeIn].nameIn
struct pnanovdb_buf_t { uint base; uint count; };
uint pnanovdb_buf_read_uint32(pnanovdb_buf_t buf, uint offset) {
    return offset/4u<buf.count ? vork_input_word(buf.base+offset/4u) : 0u;
}
uint2 pnanovdb_buf_read_uint64(pnanovdb_buf_t buf, uint offset) {
    return uint2(pnanovdb_buf_read_uint32(buf,offset),pnanovdb_buf_read_uint32(buf,offset+4u));
}
// PNanoVDB defines setters alongside its readers. Grid programs expose no setter.
void pnanovdb_buf_write_uint32(pnanovdb_buf_t buf,uint offset,uint value) {}
void pnanovdb_buf_write_uint64(pnanovdb_buf_t buf,uint offset,uint2 value) {}
#include "pnanovdb.slang"

float3 vork_dense_sample(uint slot, float3 position, uint type) {
        uint3 dims=uint3(vork_input_word(slot+1u),vork_input_word(slot+2u),vork_input_word(slot+3u));
        if (!all(isfinite(position)) || any(abs(position)>1.)) return float3(0.);
        float3 p=(position*.5+.5)*float3(dims)-.5;
        int3 origin=int3(floor(p)); float3 fraction=p-float3(origin);
        float3 result=0.; uint width=type==PNANOVDB_GRID_TYPE_FLOAT ? 1u : 3u;
        for (int z=0;z<2;z++) for (int y=0;y<2;y++) for (int x=0;x<2;x++) {
            int3 cell=clamp(origin+int3(x,y,z),int3(0),int3(dims)-1);
            uint at=slot+6u+uint(cell.x+int(dims.x)*(cell.y+int(dims.y)*cell.z))*width;
            float3 value=width==1u ? float3(asfloat(vork_input_word(at))) : float3(asfloat(vork_input_word(at)),asfloat(vork_input_word(at+1u)),asfloat(vork_input_word(at+2u)));
            float3 w=lerp(1.-fraction,fraction,float3(x,y,z));
            result+=value*w.x*w.y*w.z;
        }
        // The convex interpolant is enclosed by the reduced cell range even
        // after f32 interpolation rounding. Exterior zero is separate.
        return clamp(result,asfloat(vork_input_word(slot+4u)),asfloat(vork_input_word(slot+5u)));
}

float3 grid_sample(uint slot, float3 position, uint type) {
#if VORK_DENSE_GRIDS_ONLY
    return vork_dense_sample(slot,position,type);
#else
    if (vork_input_word(slot)==0xffffffffu) return vork_dense_sample(slot,position,type);
    pnanovdb_buf_t buf={slot+1u,vork_input_word(slot)};
    pnanovdb_grid_handle_t grid={pnanovdb_address_null()};
    let p=pnanovdb_grid_world_to_indexf(buf,grid,position);
    if (!all(isfinite(p)) || any(abs(p)>float3(2147483000.))) return float3(0.);
    int3 origin=int3(floor(p));float3 fraction=p-float3(origin);
    let root=pnanovdb_tree_get_root(buf,pnanovdb_grid_get_tree(buf,grid));
    pnanovdb_readaccessor_t accessor;
    pnanovdb_readaccessor_init(accessor,root);
    float3 result=float3(0.);
    for(int z=0;z<2;z++) for(int y=0;y<2;y++) for(int x=0;x<2;x++) {
        int3 cell=origin+int3(x,y,z);
        let address=pnanovdb_readaccessor_get_value_address(type,buf,accessor,cell);
        float3 value=type==PNANOVDB_GRID_TYPE_FLOAT ? float3(pnanovdb_read_float(buf,address)) : pnanovdb_read_vec3(buf,address);
        float3 w=lerp(1.-fraction,fraction,float3(x,y,z));
        result+=value*w.x*w.y*w.z;
    }
    return result;
#endif
}
// Positions use the medium's coordinate space; the grid's affine map converts
// them to voxel index space. Missing sparse voxels retain the grid background.
// Keep the sparse accessor once per voxel type instead of copying its tree
// traversal into every coefficient/transport call site. The typed boundary
// still lets the accessor specialize its scalar/vector storage layout.
[noinline]
public float sample_grid(FloatGrid grid,float3 position) {
    return grid_sample(vork_slot(grid),position,PNANOVDB_GRID_TYPE_FLOAT).x;
}
[noinline]
public float3 sample_grid(VectorGrid grid,float3 position) {
    return grid_sample(vork_slot(grid),position,PNANOVDB_GRID_TYPE_VEC3F);
}
