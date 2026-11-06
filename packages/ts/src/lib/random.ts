import { LCG_MULTIPLIER, LCG_INCREMENT, UINT32_RANGE, SEED_MIX_MULTIPLIER_A, SEED_MIX_MULTIPLIER_B, SEED_MIX_SHIFTS } from './constants.js';
/** Internal deterministic generator; fmix32 decorrelates adjacent author seeds before the LCG. */
export function seededRandom(seed:number):()=>number {
  if(!Number.isInteger(seed)||seed<0||seed>=UINT32_RANGE) throw new RangeError('seed must be a uint32');
  let state=seed;
  state=Math.imul(state^(state>>>SEED_MIX_SHIFTS[0]),SEED_MIX_MULTIPLIER_A);
  state=Math.imul(state^(state>>>SEED_MIX_SHIFTS[1]),SEED_MIX_MULTIPLIER_B);
  state=(state^(state>>>SEED_MIX_SHIFTS[2]))>>>0;
  return ()=>{state=(Math.imul(state,LCG_MULTIPLIER)+LCG_INCREMENT)>>>0;return state/UINT32_RANGE;};
}
