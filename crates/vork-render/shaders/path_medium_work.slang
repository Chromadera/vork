import material;
import vork_constants;
// An exhausted invocation ends its camera path and is counted at film write.
// Coverage has an independent call bound: medium coefficient credits must not
// authorize arbitrarily many expensive surface evaluations in dense cutouts.
static uint invocation_medium_steps=0u;
static uint invocation_alpha_steps=0u;
struct PathMediumWork { uint remaining; uint alpha_remaining; bool failed; };
// Watch transport progress, not a conservative static instruction estimate.
// Free-flight sampling and roulette own normal path termination; this counter
// catches runaway traversal. Its numerical limit is unchanged.
bool medium_work_step(inout PathMediumWork ledger) {
    let cost=1u;
    if(ledger.failed) {return false;}
    bool exhausted=cost>ledger.remaining;
#ifdef VORK_MEDIUM_WORK_LIMIT
    exhausted=exhausted || cost>VORK_MEDIUM_WORK_LIMIT-invocation_medium_steps;
#endif
    if(exhausted) {
        ledger.failed=true;
        return false;
    }
    invocation_medium_steps+=cost;ledger.remaining-=cost;return true;
}

bool medium_alpha_reserve(inout PathMediumWork ledger,uint calls,uint object) {
    if(ledger.failed) {return false;}
    if(calls>ledger.alpha_remaining || calls>64u-invocation_alpha_steps) {
        ledger.failed=true;
        return false;
    }
    invocation_alpha_steps+=calls;ledger.alpha_remaining-=calls;
    return medium_work_step(ledger);
}
