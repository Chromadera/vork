type Vec3 = readonly [number, number, number];
/** Internal vector operations shared by geometry generators. */
export function unit(v: readonly number[]): [number,number,number] {
  const length=Math.hypot(...v);
  if(v.length!==3 || !Number.isFinite(length) || length===0) throw new RangeError('expected a finite nonzero vec3');
  return v.map(x=>x/length) as [number,number,number];
}
export function cross(a:Vec3,b:Vec3):[number,number,number] {
  return [a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]];
}
