// Material records and texture taps (group 1). The record layout is generated.
import vork_constants;

import material_layout;
import std_opacity;

import variant;
__exported import material_bindings;
// Addresses refer to length-prefixed scene-sized bitsets in the immutable word table.
export bool links_intersection_words(uint a, uint b) {
    if(a==0u || b==0u) return false;
    let count=min(material_words[a],material_words[b]);
    for(uint i=0u;i<count;i++) if((material_words[a+1u+i]&material_words[b+1u+i])!=0u) return true;
    return false;
}


#ifdef VORK_METAL
Texture2D material_texture(uint i) {return material_textures.handles[i];}
SamplerState material_sampler(uint i) {return material_samplers.handles[i];}
#else
Texture2D material_texture(uint i) {return material_textures[NonUniformResourceIndex(i)];}
SamplerState material_sampler(uint i) {return material_samplers[NonUniformResourceIndex(i)];}
#endif
// Private invocation state: path kernels set their camera sampling density;
// raster kernels retain one-pixel texture gradients. Geometry/program
// differentials stay unscaled for tangent frames and finite-difference callbacks.
static float material_texture_scale=1.;
void set_material_texture_sample_count(uint count) {material_texture_scale=rsqrt(max(float(count),1.));}
float4 material_image(uint2 word,float2 uv,float2 dx,float2 dy) {
    return material_texture(word.x).SampleGrad(material_sampler(word.y),uv,
        dx*material_texture_scale,dy*material_texture_scale);
}
// Native Image handles address a full-width texture/sampler descriptor.
float4 material_image(uint address,float2 uv,float2 dx,float2 dy) {
    return material_image(uint2(material_words[address],material_words[address+1u]),uv,dx,dy);
}
// Texture coordinates and their screen derivatives, taken in uniform control flow so
// taps can sample conditionally with explicit gradients.
struct MaterialUv { float2 TEXCOORD_0; float2 TEXCOORD_1; float2 dx0; float2 dy0; float2 dx1; float2 dy1; float3 filter; };
// Named geometry attributes (stream.slang); each module sees only its own defines.
#ifndef VORK_ATTRIBUTES
#define VORK_ATTRIBUTES 0
#endif
#if VORK_ATTRIBUTES > 0
// The fragment's named attribute values (material_uv fills them); taps flagged
// REG_MATERIAL_ATTRIBUTE_FLAG read one.
static uint material_instance_mask;
static float4 material_attributes[VORK_ATTRIBUTES];
static float4 material_attribute_dx[VORK_ATTRIBUTES];
static float4 material_attribute_dy[VORK_ATTRIBUTES];
#endif
// Kernel-only attribute reader shared by typed programs and OpenPBR sockets.
float4 material_attribute(uint slot,uint differential) {
#if VORK_ATTRIBUTES > 0
    if(differential!=0u && (material_instance_mask & (1u<<slot))!=0u) return float4(0.);
    // Constant element accesses let the driver keep the fixed attribute ABI in
    // scalar registers instead of dynamically indexed per-thread arrays.
    [unroll] for(uint i=0u;i<VORK_ATTRIBUTES;i++) {
        if(slot==i) {return differential==0u ? material_attributes[i] : (differential==1u ? material_attribute_dx[i] : material_attribute_dy[i]);}
    }
    return float4(0.);
#else
    return float4(0.); // Unreachable: install rejects unbound attribute handles.
#endif
}
// One tap: its transformed coordinates and gradients, table word and whether its map
// turns the UV axes (then normal maps take derivative tangents), or a named attribute.
#if VORK_ATTRIBUTES > 0
struct Tap { float2 uv; float2 dx; float2 dy; float2 filter0; float2 filter1; uint2 word; bool basis; float4 attr_value; bool is_attribute; };
#else
struct Tap { float2 uv; float2 dx; float2 dy; float2 filter0; float2 filter1; uint2 word; bool basis; };
#endif
uint material_byte<V:IVariant>(uint r, uint input) {
    if (V.specialized && !input_textured<V>(input)) {return 0u;}
    return material_records[r].inputs[input/4u][input%4u];
}
Tap material_tap(uint r, uint tap, MaterialUv m) {
    let w=material_records[r].tag.y+(tap-1u)*REG_MATERIAL_TAP_WORDS;
    let flags=material_words[w+1u];let second=(flags&1u)!=0u;
#if VORK_ATTRIBUTES > 0
    if ((flags&REG_MATERIAL_ATTRIBUTE_FLAG)!=0u) {
        return Tap(float2(0.),float2(0.),float2(0.),float2(0.),float2(0.),uint2(0),false,material_attribute((flags>>8u)&255u,0u),true);
    }
#endif
    let t=float2x2(asfloat(material_words[w+2u]),asfloat(material_words[w+3u]),asfloat(material_words[w+4u]),asfloat(material_words[w+5u]));
    let offset=float2(asfloat(material_words[w+6u]),asfloat(material_words[w+7u]));
    let dx=mul(select(second,m.dx1,m.dx0),t);let dy=mul(select(second,m.dy1,m.dy0),t);
    return Tap(mul(select(second,m.TEXCOORD_1,m.TEXCOORD_0),t)+offset,dx,dy,
        sqrt(2.)*(dx*m.filter.x+dy*m.filter.y),sqrt(2.)*dy*m.filter.z,uint2(material_words[w],material_words[w+8u]),(flags&2u)!=0u
#if VORK_ATTRIBUTES > 0
        ,float4(0.),false
#endif
        );
}
float4 material_sample_one(Tap t) {
#if VORK_ATTRIBUTES > 0
    if (t.is_attribute) {return t.attr_value;}
#endif
    return material_image(t.word,t.uv,t.dx,t.dy);
}
// Four equal-weight points match the slot's sample-centre covariance. Each
// sub-tap keeps the original derivatives/LOD; their composite footprint adds
// the sample spread without treating the existing bilinear mip as a box.
bool material_filtered(Tap t) {return any(t.filter0!=float2(0.)) || any(t.filter1!=float2(0.));}
Tap material_filter_tap(Tap t,uint index) {
    t.uv+=select(index<2u,t.filter0,t.filter1)*select((index&1u)==0u,1.,-1.);
    return t;
}
float4 material_sample(Tap t) {
    if (!material_filtered(t)) {return material_sample_one(t);}
    return ((material_sample_one(material_filter_tap(t,0u))+material_sample_one(material_filter_tap(t,1u)))
           +(material_sample_one(material_filter_tap(t,2u))+material_sample_one(material_filter_tap(t,3u))))*.25;
}
float material_value(uint r, uint word) {return material_records[r].values[word/4u][word%4u];}
float3 material_constant_color(uint r, uint word) {return float3(material_value(r,word),material_value(r,word+1u),material_value(r,word+2u));}
// A scalar input: its constant times the bound channel.
float material_float<V:IVariant>(uint r, uint input, uint word, MaterialUv m) {
    let b=material_byte<V>(r,input);let value=material_value(r,word);
    if ((b&REG_MATERIAL_TAP_INDEX_MASK)==0u) {return value;}
    return value*material_sample(material_tap(r,b&REG_MATERIAL_TAP_INDEX_MASK,m))[min(b>>28u,3u)];
}
// A colour input: its constant times the bound RGB, or one channel repeated.
float3 material_color<V:IVariant>(uint r, uint input, uint word, MaterialUv m) {
    let b=material_byte<V>(r,input);let value=material_constant_color(r,word);
    if ((b&REG_MATERIAL_TAP_INDEX_MASK)==0u) {return value;}
    let texel=material_sample(material_tap(r,b&REG_MATERIAL_TAP_INDEX_MASK,m));
    return value*select((b>>28u)==4u,texel.rgb,float3(texel[min(b>>28u,3u)]));
}
// Std evaluation shared by raster and ray consumers:
// geometry_opacity, times vertex alpha for glTF records (tag z bit 2).
float material_alpha<V:IVariant>(uint r, MaterialUv m, float color_alpha) {
    let opacity=material_float<V>(r,MATERIAL_OPACITY,MATERIAL_OPACITY_WORD,m);
    return std_opacity(select((material_records[r].tag.z&REG_FLAG_VERTEX_ALPHA)!=0u,opacity*color_alpha,opacity), material_float<V>(r,MATERIAL_CUTOFF,MATERIAL_CUTOFF_WORD,m));
}
float material_constant_alpha(uint r) {
    return std_opacity(material_value(r,MATERIAL_OPACITY_WORD),material_value(r,MATERIAL_CUTOFF_WORD));
}
bool material_alpha_bound<V:IVariant>(uint r) {
    return (material_byte<V>(r,MATERIAL_OPACITY)&REG_MATERIAL_TAP_INDEX_MASK)!=0u ||
        (material_byte<V>(r,MATERIAL_CUTOFF)&REG_MATERIAL_TAP_INDEX_MASK)!=0u;
}
// Unqualified signatures serve compute passes; specialized raster paths use V.
uint material_byte(uint r,uint input) {return material_byte<Unspecialized>(r,input);}
float material_float(uint r,uint input,uint word,MaterialUv m) {return material_float<Unspecialized>(r,input,word,m);}
float3 material_color(uint r,uint input,uint word,MaterialUv m) {return material_color<Unspecialized>(r,input,word,m);}
float material_alpha(uint r,MaterialUv m,float color_alpha) {return material_alpha<Unspecialized>(r,m,color_alpha);}

// Object header word 13 points to an immutable instance-attribute row, or zero.
void material_instance_attributes(uint object_header) {
#if VORK_ATTRIBUTES > 0
    let row=material_words[object_header+13u];
    material_instance_mask=row==0u ? 0u : material_words[row];
    [unroll] for(uint slot=0u;slot<VORK_ATTRIBUTES;slot++) {
        if((material_instance_mask & (1u<<slot))!=0u) {
            let at=row+1u+slot*4u;
            material_attributes[slot]=float4(asfloat(material_words[at]),asfloat(material_words[at+1u]),asfloat(material_words[at+2u]),asfloat(material_words[at+3u]));
        }
    }
#endif
}

// Authored programs and textured imports preserve covered-sample evaluation.
bool material_sampled(uint r) {
    return material_records[r].tag.x!=0u || (material_records[r].tag.z&REG_FLAG_SAMPLE_MATERIAL)!=0u;
}
