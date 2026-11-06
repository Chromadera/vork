public struct PropagationContext {
    public float3 position;
    public float3 direction;
    public float time;
    public float previous_distance;
};
public struct PropagationStep {
    public float3 direction;
    public float distance;
    public bool absorbed = false;
    public float3 radiance = float3(1.);
};
