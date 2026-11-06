//! Shipped volume-depth writers for the camera's existing paged sample table.
//! The existing per-pass vertex record carries the installed page capacity in
//! its reserved word, avoiding device-specific shader specialization.
//! Pages are scalar words (four per sample): Metal forbids atomics on a vector
//! element, so the depth atomic addresses word 3 directly.
pub fn storage(pages: u32) -> String {
    // One binding array (a Metal argument buffer), whatever the page count:
    // a binding per page exhausted Metal's 31 buffer slots per stage.
    let mut code = String::from("#ifdef VORK_METAL\nstruct VolumeSamplePages {RWStructuredBuffer<uint> buffers[];};\n[[vk::binding(1,2)]] ParameterBlock<VolumeSamplePages> volume_sample_pages;\n#define volume_sample_page(page) volume_sample_pages.buffers[page]\n#else\n[[vk::binding(1,2)]] RWStructuredBuffer<uint> volume_sample_pages[];\n#define volume_sample_page(page) volume_sample_pages[page]\n#endif\n");
    code.push_str("export void volume_sample_record_depth(uint index,float fraction,uint capacity) {\n");
    if pages>1 {
        code.push_str("uint page=index/capacity;index=index%capacity;switch(page) {\n");
    }
    for page in 0..pages {
        if pages>1 {code.push_str(&format!("case {page}u:"));}
        code.push_str(&format!("{{uint ignored;InterlockedMax(volume_sample_page({page}u)[index*4u+3u],~asuint(clamp(fraction,0.,1.)),ignored);return;}}\n"));
    }
    if pages>1 {code.push_str("}\n");}
    code.push_str("}\n");code
}
