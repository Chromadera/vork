/** Edit distance with adjacent swaps, matching the CLI's spelling diagnostics. */
function optionDistance(left: string, right: string): number {
  const a = Array.from(left), b = Array.from(right);
  const d = Array.from({length:a.length+1}, (_, i) => Array.from({length:b.length+1}, (_, j) => i === 0 ? j : j === 0 ? i : 0));
  for (let i=1; i<=a.length; i++) for (let j=1; j<=b.length; j++) {
    d[i]![j] = Math.min(d[i-1]![j]!+1, d[i]![j-1]!+1, d[i-1]![j-1]!+Number(a[i-1] !== b[j-1]));
    if (i>1 && j>1 && a[i-1] === b[j-2] && a[i-2] === b[j-1]) d[i]![j] = Math.min(d[i]![j]!, d[i-2]![j-2]!+1);
  }
  return d[a.length]![b.length]!;
}

/** Reject misspelled helper options rather than silently using a default. */
export function checkOptions(value: object, allowed: readonly string[]): void {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new TypeError('options must be an object');
  for (const key of Object.keys(value)) if (!allowed.includes(key)) {
    const valid = [...new Set(allowed)].map(name => ({name, distance:optionDistance(key,name)}))
      .sort((a,b) => a.distance-b.distance || (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
    const nearest = valid[0];
    const suggestion = nearest && nearest.distance <= Math.max(1, Math.floor(Array.from(key).length/3))
      ? ` did you mean ${JSON.stringify(nearest.name)}?` : '';
    const label = valid.length > 5 ? `Nearest valid names (5 of ${valid.length})` : 'Valid names';
    const names = valid.length ? `${label}: ${valid.slice(0,5).map(value => value.name).join(', ')}` : 'no valid names are declared';
    throw new TypeError(`unknown option ${JSON.stringify(key)};${suggestion} ${names}`);
  }
}
/** Names have one rule across SDK generators and scene resources. */
export function checkName(name:unknown,kind='name'):asserts name is string {
  if(typeof name!=='string'||!name||name.includes('/')) throw new RangeError(`${kind} must be nonempty without /`);
}

// Scale every corner by one factor so each shared edge fits, preserving unequal radii.
// Normalize first so sums of finite radii cannot overflow. Not a std entry point.
export function clampRadii(radii: number[], edges: [number, number[]][]): number[] {
  for (const r of radii)
    if (!Number.isFinite(r) || r < 0) throw new RangeError('radius must be finite and >= 0');
  const maximum = Math.max(0, ...radii);
  if (maximum === 0) return radii.slice();
  let fitted = maximum;
  for (const [length, corners] of edges) {
    const sum = corners.reduce((total, i) => total + radii[i]! / maximum, 0);
    if (sum > 0) fitted = Math.min(fitted, length / sum);
  }
  return fitted === maximum ? radii.slice() : radii.map(r => (r / maximum) * fitted);
}
