// Bounded height field on a flat opaque floor. XZ domain faces close explicitly
// in the shared implicit traversal, retaining every physical wetness sample. The middle Y sample coincides
// with the floor, so interpolation above it represents sub-cell film depths.
// The lower positive sample closes within roundoff of that same floor; the
// renderer's existing hemisphere guard retains the exact upper-cell normal.
void compute(ComputeContext ctx,Inputs inputs) {
 uint id=ctx.id.x;if(id>=film_surface_count) return;
 uint3 c=uint3(id%film_surface_dims.x,(id/film_surface_dims.x)%film_surface_dims.y,id/(film_surface_dims.x*film_surface_dims.y));
 float2 p=DOMAIN_ORIGIN.xz-DOMAIN_EXTENT.xz+2.*DOMAIN_EXTENT.xz*(float2(c.x,c.z)+.5)/float2(film_surface_dims.xz);
 float half_height=2.*DOMAIN_EXTENT.y;
 float roundoff=8.*1.1920928955078125e-7*max(DOMAIN,max(abs(film_height(p)),1./METRES_PER_UNIT));
 float upper_height=(1.-1./float(film_surface_dims.y))*half_height;
 if(c.y==0u) {film_surface_store(id,upper_height*upper_height/roundoff);return;}
 float depth=0.;
 if(film_accept(p)) {
  float dx=2.*DOMAIN/float(wetness_dims.x);
  float2 q=(p-DOMAIN_ORIGIN.xz+DOMAIN_EXTENT.xz)/dx-.5;
  int2 lo=int2(floor(q));float2 fraction=q-float2(lo);
  for(int z=0;z<2;z++) for(int x=0;x<2;x++) {
   int2 cell=clamp(lo+int2(x,z),int2(0),int2(wetness_dims.x,wetness_dims.z)-1);
   depth+=wetness_load(uint(cell.x)+wetness_dims.x*uint(cell.y))*(x==0 ? 1.-fraction.x : fraction.x)*(z==0 ? 1.-fraction.y : fraction.y);
  }
 }
 if(depth>=upper_height) {film_surface_store(id,asfloat(0x7fc00000u));return;}
 float y=c.y==film_surface_dims.y/2u ? 0. : upper_height;
 // Keep only a strictly positive dry sentinel above the floor; the larger
 // lower-face roundoff budget must not erase representable sub-cell depths.
 film_surface_store(id,y-depth+roundoff*1.1920928955078125e-7*1.1920928955078125e-7);
}
