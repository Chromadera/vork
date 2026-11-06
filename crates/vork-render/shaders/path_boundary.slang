// Invocation-local dielectric occupancy. Capacity is the installed bound on
// solids whose bounds hold one point (passes::path::occupancy), not a quality
// limit or an authored nesting guess.
import vork_constants;
import closure;
__exported import dielectric_interface;
struct PathBoundary {
    uint object;
    uint medium; // Shared material-record handle; participating coefficients reuse it.
    float ior;
    float dispersion_scale;
    float dispersion_abbe;
    float3 color;
    float depth;
};
PathBoundary path_air() {PathBoundary air={};air.ior=1.;air.color=float3(1.);return air;}
PathBoundary path_boundary(uint object, Closure c, uint medium=0xffffffffu) {
    PathBoundary b;b.object=object;b.medium=medium;b.ior=c.specular_ior;b.dispersion_scale=c.transmission_dispersion_scale;b.dispersion_abbe=c.transmission_dispersion_abbe_number;
    b.color=c.transmission_color;b.depth=c.transmission_depth;return b;
}
DielectricSegment path_segment(PathBoundary b) {DielectricSegment s;s.color=b.color;s.depth=b.depth;s.ior=b.ior;return s;}
DielectricInterface path_interface(PathBoundary incident,PathBoundary transmitted) {
    DielectricInterface media;media.incident=path_segment(incident);media.transmitted=path_segment(transmitted);return media;
}
float3 boundary_attenuation(PathBoundary b,float distance) {return segment_attenuation(path_segment(b),distance);}
// The most recently entered active solid owns the
// refractive medium. Removing any boundary preserves all other active solids.
// For properly nested solids this is precisely the usual dielectric stack.
struct PathOccupancy<let capacity:uint> {
    PathBoundary active[capacity>0u ? capacity : 1u];
    uint count;
    // Small installed sets use constant subscripts so the compiler can keep
    // descriptors in registers. Larger sets retain compact indexed code.
    PathBoundary at(uint index) {
        if (capacity<=REG_PATH_REGISTER_BOUNDARIES) {
            PathBoundary result={};
            [unroll] for (var i=0u;i<capacity;i++) {if(i==index) {result=active[i];}}
            return result;
        }
        return active[index];
    }
    [mutating] void set(uint index,PathBoundary b) {
        if (capacity<=REG_PATH_REGISTER_BOUNDARIES) {
            [unroll] for (var i=0u;i<capacity;i++) {if(i==index) {active[i]=b;}}
        } else {active[index]=b;}
    }
    uint find(uint object) {
        if (capacity<=REG_PATH_REGISTER_BOUNDARIES) {
            var result=count;
            [unroll] for (var i=0u;i<capacity;i++) {if(i<count && active[i].object==object) {result=i;}}
            return result;
        }
        for (var i=0u;i<count;i++) {if (active[i].object==object) {return i;}}
        return count;
    }
    PathBoundary current() {
        if (count>0u) {return at(count-1u);}
        return path_air();
    }
    float ior() {return current().ior;}
    float3 attenuation(float distance) {return boundary_attenuation(current(),distance);}
    // The transmitted owner can be selected without copying or mutating the
    // whole active set. Removing a non-owner cannot change the optical medium.
    PathBoundary transmitted(PathBoundary b,bool entering) {
        if (entering && find(b.object)==count) {return b;}
        if (!entering && count>0u && at(count-1u).object==b.object) {
            if (count>1u) {return at(count-2u);}
            return path_air();
        }
        return current();
    }
    // Returns false only for a violated install bound; callers must report it.
    [mutating] bool cross(PathBoundary b, bool entering) {
        let slot=find(b.object);
        if (entering) {
            if (slot<count) {return true;}
            if (count>=capacity) {return false;}
            set(count,b);count++;
        } else if (slot<count) {remove(slot);}
        return true;
    }
    // Removing any entry preserves the order (and so the owner) of the others.
    [mutating] void remove(uint slot) {
        if (capacity<=REG_PATH_REGISTER_BOUNDARIES) {
            [unroll] for (var i=0u;i+1u<capacity;i++) {if(i>=slot && i+1u<count) {active[i]=active[i+1u];}}
        } else {
            for (var i=slot+1u;i<count;i++) {active[i-1u]=active[i];}
        }
        count--;
    }
    float relative_ior(PathBoundary b, bool entering) {
        return transmitted(b,entering).ior/ior();
    }
};
