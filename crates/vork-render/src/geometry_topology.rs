//! The topology-changing stage contract. Maps describe correspondence to the
//! immediate predecessor; composing stages composes these maps, not vertex IDs.
use crate::*;
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AttributeDomain { Vertex, Primitive, Corner, Instance, Constant }

#[derive(Clone, Copy)]
pub(crate) enum Connectivity<'a> {
    Triangles(&'a [u32]),
    Polygons { indices: &'a [u32], counts: &'a [u32] },
    Points,
}
#[derive(Clone, Copy)]
pub(crate) struct Topology<'a> {
    pub vertices: usize,
    pub connectivity: Connectivity<'a>,
}
impl Topology<'_> {
    pub(crate) fn primitives(&self) -> usize {
        match self.connectivity {
            Connectivity::Triangles(indices) => indices.len()/3,
            Connectivity::Polygons {counts,..} => counts.len(),
            Connectivity::Points => self.vertices,
        }
    }
    fn corners(&self) -> usize {
        match self.connectivity {
            Connectivity::Triangles(indices) | Connectivity::Polygons {indices,..} => indices.len(),
            Connectivity::Points => self.vertices,
        }
    }
    fn validate(&self) -> Result<()> {
        u32::try_from(self.vertices).context("geometry vertex capacity exceeds u32 addressing")?;
        u32::try_from(self.primitives()).context("geometry primitive capacity exceeds u32 addressing")?;
        let indices = match self.connectivity {
            Connectivity::Triangles(indices) => {
                ensure!(indices.len()%3==0,"triangle topology has an incomplete primitive");
                indices
            }
            Connectivity::Polygons {indices,counts} => {
                ensure!(counts.iter().all(|&count|count>=3),"polygon topology needs at least three corners per face");
                let corners=counts.iter().try_fold(0u64,|total,&count|total.checked_add(u64::from(count))).context("polygon corner count overflow")?;
                ensure!(corners==indices.len() as u64,"polygon corner counts do not cover connectivity");
                indices
            }
            Connectivity::Points => return Ok(()),
        };
        u32::try_from(indices.len()).context("geometry connectivity exceeds u32 addressing")?;
        ensure!(indices.iter().all(|&index|(index as usize)<self.vertices),"topology references a missing vertex");
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(crate) enum RowMap<'a> {
    Identity,
    Indexed(&'a [u32]),
    Stencils(&'a vork_mesh::Plan),
    Repeated { rows_per_input: usize },
}
enum Contributions<'a> {
    Single(Option<(u32,f32)>),
    Weighted(std::slice::Iter<'a,(u32,f32)>),
}
impl Iterator for Contributions<'_> {
    type Item=(u32,f32);
    fn next(&mut self) -> Option<Self::Item> {
        match self {Self::Single(value)=>value.take(),Self::Weighted(values)=>values.next().copied()}
    }
}
impl RowMap<'_> {
    fn sources(&self,row:usize) -> Contributions<'_> {
        match *self {
            Self::Identity => Contributions::Single(Some((row as u32,1.))),
            Self::Indexed(rows) => Contributions::Single(Some((rows[row],1.))),
            Self::Stencils(plan) => Contributions::Weighted(plan.stencils[row].iter()),
            Self::Repeated {rows_per_input} => Contributions::Single(Some(((row/rows_per_input) as u32,1.))),
        }
    }
    fn validate(&self,input:usize,output:usize) -> Result<()> {
        match *self {
            Self::Identity => ensure!(input==output,"identity correspondence changes row count"),
            Self::Indexed(rows) => {
                ensure!(rows.len()==output,"indexed correspondence does not cover output rows");
                ensure!(rows.iter().all(|&row|(row as usize)<input),"correspondence references a missing predecessor row");
            }
            Self::Stencils(plan) => {
                ensure!(plan.stencils.len()==output,"stencil correspondence does not cover output vertices");
                for row in &plan.stencils {
                    ensure!(!row.is_empty(),"empty vertex correspondence");
                    ensure!(row.iter().all(|&(index,weight)|(index as usize)<input && weight.is_finite()),"invalid predecessor stencil contribution");
                    // Validate affine correspondence in f64, without changing
                    // the established f32 evaluation order or normalizing it.
                    let mass=row.iter().map(|&(_,weight)|f64::from(weight)).sum::<f64>();
                    ensure!((mass-1.).abs()<1e-4,"vertex correspondence does not preserve constants");
                }
            }
            Self::Repeated {rows_per_input} => ensure!(rows_per_input>0 && input.checked_mul(rows_per_input)==Some(output),"repeated correspondence has inconsistent capacity"),
        }
        Ok(())
    }
    fn apply<'a,const N:usize>(&self,values:&'a [[f32;N]]) -> Cow<'a,[[f32;N]]> {
        match *self {
            Self::Identity => Cow::Borrowed(values),
            Self::Indexed(rows) => Cow::Owned(rows.iter().map(|&row|values[row as usize]).collect()),
            Self::Stencils(plan) => Cow::Owned(plan.evaluate(values)),
            Self::Repeated {rows_per_input} => Cow::Owned(values.iter().flat_map(|value|std::iter::repeat_n(*value,rows_per_input)).collect()),
        }
    }
}

/// A stage must supply every changed domain's map. Unowned constant/instance
/// rows pass through; an absent corner map requires vertex-indexed source seams.
#[derive(Clone, Copy)]
pub(crate) struct Transition<'a> {
    pub input: Topology<'a>,
    pub output: Topology<'a>,
    pub vertices: RowMap<'a>,
    pub primitives: RowMap<'a>,
    pub corners: Option<RowMap<'a>>,
    /// Empty means predecessor row identity; IDs are never interpolated.
    pub source_vertex_ids: &'a [u64],
}
impl Transition<'_> {
    pub(crate) fn validate(&self) -> Result<()> {
        self.input.validate()?;
        self.output.validate()?;
        ensure!(self.source_vertex_ids.is_empty() || self.source_vertex_ids.len()==self.input.vertices,
            "source identity table does not cover predecessor vertices");
        self.vertices.validate(self.input.vertices,self.output.vertices)?;
        self.primitives.validate(self.input.primitives(),self.output.primitives())?;
        if let Some(map)=self.corners {map.validate(self.input.corners(),self.output.corners())?;}
        Ok(())
    }
    pub(crate) fn vertex_sources(&self,row:usize) -> impl Iterator<Item=(u64,f32)> + '_ {
        assert!(row<self.output.vertices,"vertex outside topology contract");
        self.vertices.sources(row).map(|(source,weight)|
            (if self.source_vertex_ids.is_empty() {u64::from(source)} else {self.source_vertex_ids[source as usize]},weight))
    }
    pub(crate) fn remap<'a,const N:usize>(&self,domain:AttributeDomain,values:&'a [[f32;N]]) -> Result<Cow<'a,[[f32;N]]>> {
        let (count,map)=match domain {
            AttributeDomain::Vertex => (self.input.vertices,self.vertices),
            AttributeDomain::Primitive => (self.input.primitives(),self.primitives),
            AttributeDomain::Corner => (self.input.corners(),self.corners.context("topology stage requires vertex-indexed attributes; split seams in the source")?),
            AttributeDomain::Constant => {ensure!(values.len()==1,"constant attribute needs one row");return Ok(Cow::Borrowed(values));}
            AttributeDomain::Instance => return Ok(Cow::Borrowed(values)),
        };
        ensure!(values.len()==count,"{domain:?} attribute has {} rows; predecessor domain has {count}",values.len());
        Ok(map.apply(values))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subdivision_maps_carried_domains_and_source_correspondence() -> Result<()> {
        let indices=[0,1,2,3];let counts=[4];
        let plan=vork_mesh::Plan::new(4,&indices,Some(&counts),&[],2,true,false)?;
        let transition=Transition {input:Topology {vertices:4,connectivity:Connectivity::Polygons {indices:&indices,counts:&counts}},
            output:Topology {vertices:plan.stencils.len(),connectivity:Connectivity::Triangles(&plan.indices)},
            vertices:RowMap::Stencils(&plan),primitives:RowMap::Indexed(&plan.primitive_ids),corners:None,source_vertex_ids:&[11,13,17,19]};
        transition.validate()?;
        assert!(transition.vertex_sources(0).all(|(source,_)|[11,13,17,19].contains(&source)),"refinement fabricated a source ID");
        let constant=vec![[7.,-2.];4];
        assert!(transition.remap(AttributeDomain::Vertex,&constant)?.iter().all(|value|*value==[7.,-2.]));
        let labels=[[37.]];
        assert!(transition.remap(AttributeDomain::Primitive,&labels)?.iter().all(|value|*value==[37.]));
        let instances=[[1.,2.],[3.,4.]];
        assert!(matches!(transition.remap(AttributeDomain::Instance,&instances)?,Cow::Borrowed(_)));
        assert!(transition.remap(AttributeDomain::Corner,&[[0.];4]).is_err());
        assert!(transition.remap(AttributeDomain::Vertex,&[[0.];3]).is_err());
        // Every refined sample remains a weighted reference to the four source
        // vertices, including translation/scale rather than an invented ID.
        let values=[[0.,0.],[1.,0.],[1.,1.],[0.,1.]];
        let mapped=transition.remap(AttributeDomain::Vertex,&values)?;
        let translated=values.map(|value|[value[0]*2.+3.,value[1]*2.-5.]);
        let moved=transition.remap(AttributeDomain::Vertex,&translated)?;
        assert!(mapped.iter().zip(moved.iter()).all(|(a,b)|b==&[a[0]*2.+3.,a[1]*2.-5.]));
        Ok(())
    }
    #[test]
    fn invalid_topology_and_correspondence_are_refused() -> Result<()> {
        let mut plan=vork_mesh::Plan::new(3,&[0,1,2],None,&[],1,true,false)?;
        plan.stencils[0]=vec![(3,1.)];
        let transition=Transition {input:Topology {vertices:3,connectivity:Connectivity::Triangles(&[0,1,2])},
            output:Topology {vertices:plan.stencils.len(),connectivity:Connectivity::Triangles(&plan.indices)},
            vertices:RowMap::Stencils(&plan),primitives:RowMap::Indexed(&plan.primitive_ids),corners:None,source_vertex_ids:&[]};
        assert!(transition.validate().is_err());
        plan.stencils[0]=vec![(0,f32::NAN)];
        assert!(RowMap::Stencils(&plan).validate(3,plan.stencils.len()).is_err());
        assert!(Topology {vertices:3,connectivity:Connectivity::Triangles(&[0,1,3])}.validate().is_err());
        assert!(Topology {vertices:4,connectivity:Connectivity::Polygons {indices:&[0,1,2,3],counts:&[3]}}.validate().is_err());
        let points=Topology {vertices:2,connectivity:Connectivity::Points};points.validate()?;
        assert_eq!(points.primitives(),2);
        RowMap::Repeated {rows_per_input:3}.validate(2,6)?;
        assert!(matches!(RowMap::Identity.apply(&[[1.]]),Cow::Borrowed(_)));
        Ok(())
    }
}
