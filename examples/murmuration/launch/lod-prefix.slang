struct Inputs {};
void compute(ComputeContext c, Inputs i) {
 uint sum=0;
 [loop] for(uint block=0;block<lod_counts_count;block++) {
  lod_offsets_store(block,sum);sum+=lod_counts_load(block);
 }
 near_count_store(0,min(sum,32768u));
}
