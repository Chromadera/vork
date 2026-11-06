//! Scene-sized named light-link bitsets in the shared immutable material word table.
use super::*;
use ::vork_scene::v2;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy)]
pub(crate) struct Selection {
    /// Word address of a length-prefixed bitset; zero is the empty set.
    pub mask: u32,
    pub all: bool,
}
const ALL: Selection = Selection { mask: 0, all: true };
pub(crate) struct Links {
    names: BTreeMap<String, usize>,
    pub groups: Vec<u32>,
    illuminates: Vec<Selection>,
    shadowed_by: Vec<Selection>,
    words: Vec<u32>,
    sets: BTreeMap<Vec<u32>, u32>,
    base: u32,
}
impl Links {
    fn intern(&mut self, bits: Vec<u32>) -> Result<u32> {
        if bits.iter().all(|&word| word == 0) {return Ok(0);}
        if let Some(&address) = self.sets.get(&bits) {return Ok(address);}
        let address = u32::try_from(self.words.len())?;
        self.words.push(u32::try_from(bits.len())?);
        self.words.extend_from_slice(&bits);
        self.sets.insert(bits, address);
        Ok(address)
    }
    fn bits(&self, address: u32) -> &[u32] {
        if address == 0 {return &[];}
        let at = (address - self.base) as usize;
        &self.words[at+1..at+1+self.words[at] as usize]
    }
    fn union(&mut self, parent: u32, names: &[String]) -> Result<u32> {
        let mut bits = vec![0; self.names.len().div_ceil(32)];
        let inherited = self.bits(parent);
        bits[..inherited.len()].copy_from_slice(inherited);
        for name in names {
            let index = self.names[name];
            bits[index/32] |= 1 << (index%32);
        }
        self.intern(bits)
    }
    fn selection(&mut self, names: Option<&[String]>) -> Result<Selection> {
        Ok(Selection {mask: self.union(0, names.unwrap_or_default())?, all: names.is_none()})
    }
    #[cfg(test)]
    fn intersects(&self, a: u32, b: u32) -> bool {
        self.bits(a).iter().zip(self.bits(b)).any(|(a,b)| a&b != 0)
    }
    pub fn illuminates(&self, node: usize) -> Selection {self.illuminates.get(node).copied().unwrap_or(ALL)}
    pub fn shadowed_by(&self, node: usize) -> Selection {self.shadowed_by.get(node).copied().unwrap_or(ALL)}
    pub fn new(nodes: &[EvalNode]) -> Result<Self> {
        let mut names = BTreeSet::new();
        for node in nodes.iter().map(|n| &n.node) {
            names.extend(node.light_link_tags.iter().cloned());
            if let v2::NodeKind::Asset { nodes, .. } = &node.kind {
                for node in nodes.values() {names.extend(node.light_link_tags.iter().flatten().cloned());}
            }
        }
        let names = names.into_iter().enumerate().map(|(i,name)|(name,i)).collect();
        let mut result = Self {names, groups:vec![0;nodes.len()], illuminates:vec![ALL;nodes.len()], shadowed_by:vec![ALL;nodes.len()], words:vec![0], sets:BTreeMap::new(), base:0};
        for (i,node) in nodes.iter().enumerate() {
            let parent = node.parent.map_or(0,|p|result.groups[p]);
            result.groups[i] = result.union(parent, &node.node.light_link_tags)?;
            if let v2::NodeKind::Light {illuminates,shadowed_by,..} = &node.node.kind {
                result.illuminates[i] = result.selection(illuminates.as_deref())?;
                result.shadowed_by[i] = result.selection(shadowed_by.as_deref())?;
            }
        }
        Ok(result)
    }
    pub fn object_groups(&mut self, root: usize, source: &v2::Node, path: &[(usize,String)]) -> Result<u32> {
        let mut groups = self.groups[root];
        if let v2::NodeKind::Asset {nodes,..} = &source.kind {
            for (index,name) in path {
                if let Some(own) = buffers::imported_override(nodes,*index,name).and_then(|n|n.light_link_tags.as_ref()) {
                    groups = self.union(groups,own)?;
                }
            }
        }
        Ok(groups)
    }
    /// Relocate every handle once, after all imported memberships are interned.
    pub fn finish(&mut self, words: &mut Vec<u32>, record_words: u32, objects: &mut [Object]) -> Result<()> {
        self.base = record_words.checked_add(u32::try_from(words.len())?).context("light-link word address exceeds u32")?;
        self.base.checked_add(u32::try_from(self.words.len())?).context("light-link word table exceeds u32")?;
        let relocate = |address: &mut u32| {if *address != 0 {*address += self.base;}};
        for group in &mut self.groups {relocate(group);}
        for selection in self.illuminates.iter_mut().chain(&mut self.shadowed_by) {relocate(&mut selection.mask);}
        for object in objects {relocate(&mut object.metadata[0]);}
        words.extend_from_slice(&self.words);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arbitrary_width_memberships_and_wildcards() -> Result<()> {
        let nodes:Vec<v2::Node> = vork_scene::v2::from_test_value(serde_json::json!([
            {"type":"node", "light_link_tags":(0..130).map(|i|format!("g{i}")).collect::<Vec<_>>()},
            {"type":"light","shape":"point","illuminates":["g129"]},
            {"type":"node"},
            {"type":"node","light_link_tags":["g1"]}
        ]))?;
        let nodes:Vec<_> = nodes.into_iter().enumerate().map(|(i,node)|EvalNode { camera: None,node,parent:(i==2).then_some(0),world:Mat4::IDENTITY,dynamic:false,embedded:None}).collect();
        let mut links = Links::new(&nodes)?;
        assert!(links.intersects(links.groups[2],links.illuminates(1).mask));
        assert!(!links.intersects(links.groups[3],links.illuminates(1).mask));
        assert!(links.illuminates(0).all);
        assert!(!links.illuminates(1).all);
        let mut words=vec![0;17];
        links.finish(&mut words,64,&mut [])?;
        assert!(links.intersects(links.groups[2],links.illuminates(1).mask));
        assert!(!links.intersects(links.groups[3],links.illuminates(1).mask));
        Ok(())
    }
}
