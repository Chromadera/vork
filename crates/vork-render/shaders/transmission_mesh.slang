// Mesh exits retained at install; geometry and current pose share the camera hit buffers.
import hit_geometry;
import mesh_exit;
import frame;
import vork_constants;
struct SolidGeometry<G:IHitGeometry> : IExitGeometry {
    uint buffer;uint3 positions;bool deformed;uint2 base;
    __init(uint buffer,uint3 positions,bool deformed,uint2 base) {this.buffer=buffer;this.positions=positions;this.deformed=deformed;this.base=base;}
    bool bounded() {return !deformed;}
    uint mesh_word(uint index) {return G.load(buffer,index);}
    uint4 mesh_vec(uint index) {return uint4(mesh_word(index),mesh_word(index+1u),mesh_word(index+2u),mesh_word(index+3u));}
    uint triangle(uint triangle) {return base.y+triangle*REG_EXIT_TRIANGLE_VEC4S*4u*(!deformed ? 2u : 1u);}
    uint4 word(uint node,uint field) {return mesh_vec(base.x+(node*REG_EXIT_NODE_VEC4S+field)*4u);}
    float3 vertex(uint triangle,uint corner) {
        let first=this.triangle(triangle);
        if (!deformed) {return hit_vec3<G>(buffer,first+(REG_EXIT_TRIANGLE_VEC4S+corner)*4u);}
        return hit_vec3<G>(positions.x,positions.y+mesh_word(first+corner*4u+3u)*positions.z);
    }
};
SolidExit mesh_solid_exit<G:IHitGeometry>(uint draw,uint object,float3 origin,float3 direction,float eta) {
    let buffer=G.word(draw*REG_RAY_SLOT_WORDS);
    if (buffer==0xffffffffu) {return SolidExit(REG_FP32_MAX,float3(0.),false);}
    let normal=object_normal(object);let inverse=transpose(normal);
    let p=mul(float4(origin,1.),inverse).xyz;let d=mul(float4(direction,0.),inverse).xyz;
    let record=G.word(draw*REG_RAY_SLOT_WORDS+2u);
    let positions=uint3(G.word(record),G.word(record+1u),G.word(record+2u));
    let deformed=(G.word(record+8u)>>24u)!=0u;
    let count=G.load(buffer,0u);
    let base=uint2(1u,1u+count*REG_EXIT_NODE_VEC4S*4u);
    let geometry=SolidGeometry<G>(buffer,positions,deformed,base);
    let hit=mesh_exit<SolidGeometry<G>>(geometry,count,p,d,REG_FP32_MAX,0.);
    if (!hit.found) {return SolidExit(hit.distance,float3(0.),false);}
    let first=geometry.triangle(hit.triangle);
    float3 normals[3];
    for (uint i=0u;i<3u;i++) {
        if (deformed) {
            let vertex=geometry.mesh_word(first+i*4u+3u);
            normals[i]=hit_vec3<G>(positions.x,positions.y+vertex*positions.z+3u);
        } else {normals[i]=hit_vec3<G>(buffer,first+i*4u);}
    }
    let b=hit.barycentric;
    let interpolated=normals[0]*(1.-b.x-b.y)+normals[1]*b.x+normals[2]*b.y;
    let a=geometry.vertex(hit.triangle,0u);
    let face=exit_face(a,geometry.vertex(hit.triangle,1u),geometry.vertex(hit.triangle,2u),(float3x3)object_model(object));
    let n=solid_exit_normal(interpolated,face,(float3x3)normal,direction,eta);
    return SolidExit(hit.distance,n,true);
}
