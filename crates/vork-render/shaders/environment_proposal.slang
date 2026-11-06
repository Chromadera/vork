// Lat-long radiance proposal, uniform in solid angle within each selected cell.
// The actual float CDF increments define its PDF, including zero-mass cells.
import vork_constants;
import vork_prelude;
float environment_proposal_pdf(StructuredBuffer<float2> distribution,uint2 size,float3 direction) {
    let uv=equirect_uv(direction);
    let cell=min(uint2(float2(frac(uv.x),clamp(uv.y,0.,1.))*float2(size)),size-1u);
    let index=cell.y*size.x+cell.x;
    let previous=index>0u ? distribution[index-1u].x : 0.;
    let mass=distribution[index].x-previous;
    let angle=REG_PI/float(size.y);
    let solid_angle=REG_TAU/float(size.x)*2.*sin(angle*(float(cell.y)+.5))*sin(.5*angle);
    return mass/solid_angle;
}
// The first of `count` entries first, first+stride, ... whose cumulative value,
// relative to `base`, exceeds `target`: zero-mass entries are never chosen.
uint environment_cdf_search(StructuredBuffer<float2> distribution,uint first,uint stride,uint count,float base,float target) {
    var low=0u;var high=count-1u;
    while (low<high) {
        let mid=(low+high)/2u;
        if (distribution[first+mid*stride].x-base<=target) {low=mid+1u;} else {high=mid;}
    }
    return low;
}
// Inverts the marginal row CDF with u.y, then the row's conditional column CDF
// with u.x, as Cycles does: 2D-stratified points stay stratified across cells.
// Row ends of the one row-major table are the marginal; the remainders place the
// direction inside the cell, uniformly in solid angle.
float3 environment_proposal_sample(StructuredBuffer<float2> distribution,uint2 size,float2 u, out float pdf) {
    let row=environment_cdf_search(distribution,size.x-1u,size.x,size.y,0.,u.y);
    let row_start=row>0u ? distribution[row*size.x-1u].x : 0.;
    let row_mass=distribution[(row+1u)*size.x-1u].x-row_start;
    // Relative to the row start: adding u.x*row_mass to a large global prefix
    // would round away the conditional coordinate. Below the row's mass, so
    // rounding never lands on trailing empty cells.
    let target=min(u.x*row_mass,screen_next_float(row_mass,false));
    let column=environment_cdf_search(distribution,row*size.x,1u,size.x,row_start,target);
    let at=row*size.x+column;
    let previous=(at>0u ? distribution[at-1u].x : 0.)-row_start;
    let fraction=(target-previous)/(distribution[at].x-row_start-previous);
    let height=(u.y-row_start)/row_mass;
    let phi=REG_TAU*((float(column)+fraction)/float(size.x)-.5);
    let cosine=lerp(cos(REG_PI*float(row)/float(size.y)),cos(REG_PI*float(row+1u)/float(size.y)),height);
    let sine=sqrt(max(1.-cosine*cosine,0.));
    let angle=REG_PI/float(size.y);
    let solid_angle=REG_TAU/float(size.x)*2.*sin(angle*(float(row)+.5))*sin(.5*angle);
    pdf=(distribution[at].x-(at>0u ? distribution[at-1u].x : 0.))/solid_angle;
    return float3(sine*cos(phi),cosine,sine*sin(phi));
}
