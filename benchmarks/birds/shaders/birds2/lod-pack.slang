struct Inputs {};
void compute(ComputeContext c, Inputs i) {
 uint block=c.id.x;uint index=lod_offsets_load(block);
 [loop] for(uint id=block*256;id<min((block+1)*256,1000000u);id++) {
  if(far_scale_load(id).x==0) {
   if(index<32768u) {
    near_translation_store(index,translation_load(id));near_rotation_store(index,rotation_load(id));near_scale_store(index,float3(.1466666667));
   } else {far_scale_store(id,float3(1));}
   index++;
  }
 }
}
