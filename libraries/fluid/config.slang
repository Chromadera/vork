// Included after physical constants supplied by fluid.build (or by the author).
// Cubic cells; rectangular domains use proportional grid dimensions.
uint bin_index(int3 p) {return uint(p.x)+BIN_DIMS.x*(uint(p.y)+BIN_DIMS.y*uint(p.z));}
void query_bins(float3 p,float radius,out int3 lo,out int3 hi) {
 lo=max(int3(floor((p-radius-DOMAIN_ORIGIN+DOMAIN_EXTENT)*(.5*float(BIN_RES)/DOMAIN))),int3(0));
 hi=min(int3(floor((p+radius-DOMAIN_ORIGIN+DOMAIN_EXTENT)*(.5*float(BIN_RES)/DOMAIN))),int3(BIN_DIMS)-1);
}
