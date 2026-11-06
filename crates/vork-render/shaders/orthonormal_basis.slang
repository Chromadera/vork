// Duff et al. 2017 orthonormal frame; rows store the basis vectors.
public float3x3 orthonormal_basis(float3 n) {
    let s=select(n.z>=0.,1.,-1.);let a=-1./(s+n.z);let b=n.x*n.y*a;
    return float3x3(float3(1.+s*n.x*n.x*a,s*b,-s*n.x),float3(b,s+n.y*n.y*a,-n.y),n);
}
public float3 basis_direction(float3 n,float3 v) {
    let basis=orthonormal_basis(n);
    return v.x*basis[0]+v.y*basis[1]+v.z*n;
}
