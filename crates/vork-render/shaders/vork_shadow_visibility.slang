import vork_constants;
import vork_prelude;
import variant;
#ifdef VORK_VOLUME_BINDINGS
[[vk::binding(66,0)]] public StructuredBuffer<uint> shadow_sequence;
[[vk::binding(44,0)]] public StructuredBuffer<Light> lights;
#else
import frame;
import rays;
#endif
public LightValues light_values<V:IVariant>(uint i) {return specialized_light<V>(lights[i]);}
