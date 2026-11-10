import material;
import vork_constants;
// Coverage has an independent call bound: medium coefficient credits must not
// authorize arbitrarily many expensive surface evaluations in dense cutouts.
struct PathMediumWork { uint remaining; uint alpha_remaining; bool failed; };
bool medium_work_reserve(inout PathMediumWork ledger,uint cost,uint operation,uint program) {
    if(ledger.failed) {return false;}
    if(cost>ledger.remaining) {
        ledger.failed=true;
        InterlockedMax(shader_errors[0],REG_PATH_MEDIUM_WORK);
        InterlockedMin(shader_errors[1],(operation<<24u)|(program&0x00ffffffu));
        return false;
    }
    ledger.remaining-=cost;return true;
}

bool medium_alpha_reserve(inout PathMediumWork ledger,uint calls,uint object) {
    if(ledger.failed) {return false;}
    if(calls>ledger.alpha_remaining) {
        ledger.failed=true;
        InterlockedMax(shader_errors[0],REG_PATH_MEDIUM_WORK);
        InterlockedMin(shader_errors[1],(8u<<24u)|(object&0x00ffffffu));
        return false;
    }
    ledger.alpha_remaining-=calls;
    return medium_work_reserve(ledger,calls,8u,object);
}
