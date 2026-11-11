// Blender material-buffer curve contract; shared colour maths lives in std.
__exported import std.color;
import vork;

// Exact piecewise-linear segments emitted by the Blender material compiler.
// The FloatGrid binding retains the ordinary material-word upload/relocation;
// its dense [4, segment_count, 1] array stores (x, inverse_width, y0, y1).
// Values are fetched directly, without grid interpolation or texture encoding.
public float curve_segments(FloatGrid segments, float x) {
    uint base=vork_slot(segments);
    uint count=vork_input_word(base+2u);
    if(vork_input_word(base)!=0xffffffffu || vork_input_word(base+1u)!=4u ||
       vork_input_word(base+3u)!=1u || count==0u) return 0.;
    uint left=0u,right=count;
    [loop] while(left+1u<right) {
        uint mid=left+(right-left)/2u;
        if(x<asfloat(vork_input_word(base+6u+4u*mid))) right=mid;
        else left=mid;
    }
    uint at=base+6u+4u*left;
    float lo=asfloat(vork_input_word(at));
    float inverse=asfloat(vork_input_word(at+1u));
    float a=asfloat(vork_input_word(at+2u));
    float b=asfloat(vork_input_word(at+3u));
    return lerp(a,b,(x-lo)*inverse);
}
