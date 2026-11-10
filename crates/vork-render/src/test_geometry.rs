//! Mesh fixtures authored directly as ordinary scene buffers.
use serde_json::{Value,json};

fn mesh(p:Vec<[f64;3]>,n:Vec<[f64;3]>,uv:Vec<[f64;2]>,indices:Vec<u32>)->Value {
    json!({"attributes":{"POSITION":{"type":"vec3","data":p},"NORMAL":{"type":"vec3","data":n},
        "TEXCOORD_0":{"type":"vec2","data":uv}},"indices":{"type":"u32","data":indices}})
}

pub fn latitude_longitude(radius:f64,sectors:u32,rings:u32)->Value {
    let(mut p,mut n,mut uv,mut indices)=(vec![],vec![],vec![],vec![]);
    for y in 0..=rings {for x in 0..=sectors {
        let a=f64::from(y)*std::f64::consts::PI/f64::from(rings);
        let b=f64::from(if x==sectors {0}else{x})*std::f64::consts::TAU/f64::from(sectors);
        let normal=if y==0 {[0.,1.,0.]}else if y==rings {[0.,-1.,0.]}else{[a.sin()*b.cos(),a.cos(),a.sin()*b.sin()]};
        p.push(normal.map(|v|v*radius));n.push(normal);uv.push([1.-f64::from(x)/f64::from(sectors),f64::from(y)/f64::from(rings)]);
    }}
    for y in 0..rings {for x in 0..sectors {let a=y*(sectors+1)+x;let b=a+sectors+1;
        if y<rings-1 {indices.extend([a,b+1,b]);}if y>0 {indices.extend([a,a+1,b+1]);}
    }}
    mesh(p,n,uv,indices)
}

pub fn plane(size:[f64;2],cells:[u32;2])->Value {
    let(mut p,mut n,mut uv,mut indices)=(vec![],vec![],vec![],vec![]);let [nx,nz]=cells;
    for z in 0..=nz {for x in 0..=nx {let u=f64::from(x)/f64::from(nx);let v=f64::from(z)/f64::from(nz);
        p.push([(u-0.5)*size[0],0.,(v-0.5)*size[1]]);n.push([0.,1.,0.]);uv.push([u,v]);
    }}
    for z in 0..nz {for x in 0..nx {let a=z*(nx+1)+x;let b=a+nx+1;indices.extend([a,b,a+1,a+1,b,b+1]);}}
    mesh(p,n,uv,indices)
}
