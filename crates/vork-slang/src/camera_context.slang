// Film coordinates include the pixel filter sample; all frame vectors are world-space.
public struct CameraContext {
    public float2 film_size;
    public float2 uv;
    public float2 lens_sample;
    public float time;
    public float focus_distance;
    public float3 origin;
    public float3 right;
    public float3 up;
    public float3 forward;
};
public struct CameraRay {
    public float3 origin;
    public float3 direction;
    public float t_min;
    public float t_max;
    public float3 weight = float3(1.);
    public float wavelength_scale = 1.;
};
public struct SensorContext {
    public float path_length;
    public float wavelength;
    public float2 uv;
    public float time;
};

// A projective acceleration certificate, evaluated with the same Inputs as camera().
// screen: NDC x/y scale and translation; clip: near, far, orthographic, enabled.
// aperture: radius in metres, focus depth in metres, blade count, rotation radians.
// shape: aperture x/y ratio. enabled=0 explicitly declares no projective calibration.
public struct CameraCalibration {
    public float4 screen = float4(0.);
    public float4 clip = float4(0.);
    public float4 aperture = float4(0.);
    public float4 shape = float4(1., 0., 0., 0.);
};
