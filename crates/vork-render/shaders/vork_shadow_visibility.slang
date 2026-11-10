import vork_constants;
import vork_prelude;
import variant;
import frame;
import rays;
public LightValues light_values<V:IVariant>(uint i) {return specialized_light<V>(lights[i]);}
