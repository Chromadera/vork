// Value shared by medium integration and froxel reconstruction. Finite kernels
// retain optical depth directly, even when transmission underflows. Infinite
// closures use source/transmission only.
public struct MediumTransport { public float3 source; public float3 transmission; public float3 optical_depth; };
// A fully extinguished channel cannot receive a later source, including an
// unrepresentable source in a hidden, extremely dense interval.
public float3 medium_received_source(float3 transmission,float3 source) {
    return select(transmission==float3(0.),float3(0.),transmission*source);
}
public float medium_opacity(float3 t,float alpha) {return clamp(1.-dot(t,float3(1./3.))*(1.-alpha),0.,1.);}

// Endpoints describe physical transport, independently of camera clipping.
public enum MediumEndpointKind { Receiver, Emitter, Infinity };
public struct MediumEndpoint { public MediumEndpointKind kind; public float distance; };
public MediumEndpoint medium_receiver(float distance) {return MediumEndpoint(MediumEndpointKind.Receiver,distance);}
public MediumEndpoint medium_emitter(float distance) {return MediumEndpoint(MediumEndpointKind.Emitter,distance);}
public MediumEndpoint medium_infinity() {return MediumEndpoint(MediumEndpointKind.Infinity,0.);}
