// Probe interpolation owns weights; each shading consumer supplies its existing
// geometry/coverage service for receiver-to-probe connections.
public interface IProbeVisibility {
    static const bool probe_rays;
    static bool probe_visible(float3 world,float3 normal,float3 position_error,float3 probe);
}
// The retained depth-moment approximation used by low-frequency reflections.
public struct ProbeMomentVisibility : IProbeVisibility {
    static const bool probe_rays=false;
    static bool probe_visible(float3 world,float3 normal,float3 position_error,float3 probe) {return true;}
};
