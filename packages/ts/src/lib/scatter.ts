import type { Scene, Node } from '../schema.js';
import type { GeometryFragment } from './shapes.js';
import { checkName, checkOptions } from './validate.js';
import { seededRandom } from './random.js';
import { unit, cross } from './math.js';
type Vec3 = readonly [number, number, number];

type Mesh = Extract<Node, {type: 'mesh'}>;
export type InstanceFragment = {buffers: NonNullable<Scene['buffers']>; instances: NonNullable<Mesh['instances']>};

function positiveInteger(value: number, field: string): number {
  if (!Number.isSafeInteger(value) || value < 1) throw new RangeError(`${field} must be a positive safe integer`);
  return value;
}
function finish(name: string, transforms: number[][]): InstanceFragment {
  checkName(name, 'instance name');
  const translation = `${name}_translation`, rotation = `${name}_rotation`;
  const quaternions = transforms.map(m => {
    const diagonal = [m[0]!,m[5]!,m[10]!];
    const trace = diagonal[0]!+diagonal[1]!+diagonal[2]!;
    const q = [0,0,0,0];
    if (trace > 0) {
      const s = 2*Math.sqrt(1+trace);
      q[0]=(m[6]!-m[9]!)/s; q[1]=(m[8]!-m[2]!)/s; q[2]=(m[1]!-m[4]!)/s; q[3]=s/4;
    } else {
      const i = diagonal.indexOf(Math.max(...diagonal)), j=(i+1)%3, k=(i+2)%3;
      const s = 2*Math.sqrt(1+diagonal[i]!-diagonal[j]!-diagonal[k]!);
      q[i]=s/4; q[j]=(m[j*4+i]!+m[i*4+j]!)/s; q[k]=(m[k*4+i]!+m[i*4+k]!)/s;
      q[3]=(m[j*4+k]!-m[k*4+j]!)/s;
    }
    const length=Math.hypot(...q);
    return q.map(v=>v/length);
  });
  return {buffers: {
    [translation]: {type:'vec3', data:transforms.map(m=>m.slice(12,15))},
    [rotation]: {type:'vec4', data:quaternions},
  }, instances:{attributes:{TRANSLATION:translation,ROTATION:rotation}}};
}
/** Uniform area sampling over one inline triangle geometry; local +Y follows the face normal.
 * Transforms use the source geometry's local space; give the instanced node the surface
 * node's transform, or parent it under that node. The seed is a uint32. */
export function scatter(onGeometry: GeometryFragment, count: number, seed: number, name?:string): InstanceFragment {
  positiveInteger(count, 'count');
  const random = seededRandom(seed);
  const geometries = Object.values(onGeometry.geometry);
  if (geometries.length !== 1) throw new RangeError('scatter requires exactly one geometry');
  const geometry = geometries[0]!;
  name ??= `${Object.keys(onGeometry.geometry)[0]!}_scatter`;
  checkName(name, 'instance name');
  if (geometry.topology !== undefined && geometry.topology !== 'triangles') throw new RangeError('scatter requires triangle topology');
  const position = geometry.attributes.POSITION!;
  const source = typeof position === 'string' ? onGeometry.buffers[position] : position;
  if (!source || source.type !== 'vec3' || !source.data || source.file) throw new RangeError('scatter requires inline vec3 positions');
  const numbers = (value: unknown): number[] => {
    if (typeof value === 'number' && Number.isFinite(value)) return [value];
    if (Array.isArray(value)) return value.flatMap(numbers);
    throw new RangeError('scatter buffers require finite numeric data');
  };
  if ((source.offset ?? 0) !== 0 || source.count != null) throw new RangeError('inline scatter buffers cannot have offset/count');
  const positionRows = Array.from(source.data as readonly unknown[], numbers);
  if (!positionRows.every(row => row.length === 3)) throw new RangeError('scatter positions require vec3 elements');
  const positions = positionRows.flat();
  if (positions.length % 3 !== 0) throw new RangeError('scatter positions require complete vec3 elements');
  let indices: number[];
  if (geometry.indices) {
    const buffer = typeof geometry.indices === 'string' ? onGeometry.buffers[geometry.indices] : geometry.indices;
    if (!buffer || buffer.type !== 'u32' || !buffer.data || buffer.file) throw new RangeError('scatter requires inline u32 indices');
    if ((buffer.offset ?? 0) !== 0 || buffer.count != null) throw new RangeError('inline scatter buffers cannot have offset/count');
    const rows = Array.from(buffer.data as readonly unknown[], numbers);
    if (!rows.every(row => row.length === 1)) throw new RangeError('scatter indices require scalar elements');
    indices = rows.flat();
  } else indices = Array.from({length:positions.length/3}, (_,i) => i);
  if (indices.length % 3 !== 0 || !indices.every(i => Number.isInteger(i) && i >= 0 && i < positions.length/3))
    throw new RangeError('scatter indices require complete triangles of valid vertex indices');
  const vertex = (i:number):Vec3 => positions.slice(i*3,i*3+3) as [number,number,number];
  const sub = (a:Vec3,b:Vec3):Vec3 => a.map((v,k) => v-b[k]!) as [number,number,number];
  const triangles: {a:Vec3;b:Vec3;c:Vec3;normal:Vec3;end:number}[] = [];
  let area = 0;
  for (let i=0;i<indices.length;i+=3) {
    const a=vertex(indices[i]!),b=vertex(indices[i+1]!),c=vertex(indices[i+2]!);
    const normal=cross(sub(b,a),sub(c,a)), weight=Math.hypot(...normal)/2;
    if (!Number.isFinite(weight)) throw new RangeError('scatter triangle area is not finite');
    if (weight === 0) continue;
    area += weight;
    if (!Number.isFinite(area)) throw new RangeError('scatter surface area is not finite');
    triangles.push({a,b,c,normal:unit(normal),end:area});
  }
  if (area === 0) throw new RangeError('scatter requires nonzero surface area');
  const transforms:number[][]=[];
  for (let i=0;i<count;i++) {
    const target=random()*area;
    let lo=0,hi=triangles.length-1;
    while (lo<hi) { const mid=(lo+hi)>>>1; if (triangles[mid]!.end<=target) lo=mid+1; else hi=mid; }
    const {a,b,c,normal}=triangles[lo]!;
    const u=Math.sqrt(random()),v=random();
    const p=a.map((x,k) => x*(1-u)+b[k]!*u*(1-v)+c[k]!*u*v) as [number,number,number];
    const reference:Vec3 = Math.abs(normal[0]) < Math.abs(normal[2]) ? [1,0,0] : [0,0,1];
    const right=unit(cross(normal,reference)), forward=cross(right,normal);
    transforms.push([...right,0,...normal,0,...forward,0,...p,1]);
  }
  return finish(name, transforms);
}
