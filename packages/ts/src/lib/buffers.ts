import { createHash } from 'node:crypto';
import type { Scene } from '../schema.js';
import { BINARY_BUFFER_MIN_BYTES } from './constants.js';

type SceneBuffer = NonNullable<Scene['buffers']>[string];
const widths: Record<SceneBuffer['type'],number> = {f32:1,i32:1,u32:1,vec2:2,vec3:3,vec4:4,uvec4:4,mat4:16};

/** Encoding checks apply to externalized arrays; inline arrays remain the renderer's validation responsibility.
 * Mutates only the writer's clone. */
export function externalBuffers(scene: Scene, file: string): Uint8Array | undefined {
  const parts: Uint8Array[] = [];
  let offset=0;
  const shared=new Map<string,{packed:Uint8Array,offset:number}>();
  for(const [name,buffer] of bufferObjects(scene)) {
    if (!buffer.data) continue;
    const width=widths[buffer.type],bytes=buffer.data.length*width*4;
    if(bytes<BINARY_BUFFER_MIN_BYTES) continue;
    if(buffer.file || (buffer.offset ?? 0)!==0 || buffer.count!=null)
      throw new RangeError(`buffer ${name}: inline data cannot have file, offset or count`);
    const dims=buffer.dims;
    if(dims && (dims.length!==3 || dims.some(n=>!Number.isInteger(n)||n<=0) || dims.reduce((a,b)=>a*b,1)!==buffer.data.length))
      throw new RangeError(`buffer ${name}: data length must match positive dims product`);
    const packed=new Uint8Array(bytes),view=new DataView(packed.buffer);
    let at=0;
    for(const row of buffer.data as readonly unknown[]) {
      let components=0;
      const write=(value:unknown):void => {
        if(components++>=width) throw new RangeError(`buffer ${name}: element width must be ${width}`);
        if(typeof value!=='number'||!Number.isFinite(value)) throw new RangeError(`buffer ${name}: expected finite numbers`);
        if(buffer.type==='u32'||buffer.type==='uvec4') {
          if(!Number.isInteger(value)||value<0||value>0xffffffff) throw new RangeError(`buffer ${name}: expected u32`);
          view.setUint32(at,value,true);
        } else if(buffer.type==='i32') {
          if(!Number.isInteger(value)||value< -0x80000000||value>0x7fffffff) throw new RangeError(`buffer ${name}: expected i32`);
          view.setInt32(at,value,true);
        } else {
          if(!Number.isFinite(Math.fround(value))) throw new RangeError(`buffer ${name}: expected finite f32`);
          view.setFloat32(at,value,true);
        }
        at+=4;
      };
      visitScalars(row,write);
      if(components!==width) throw new RangeError(`buffer ${name}: element width must be ${width}`);
    }
    const key=createHash('sha256').update(packed).digest('hex');
    const previous=shared.get(key);
    const same=previous?.packed.length===packed.length && packed.every((v,i)=>v===previous.packed[i]);
    const atOffset=same?previous!.offset:offset;
    const count=buffer.data.length;
    Reflect.deleteProperty(buffer,'data');
    Object.assign(buffer,{file,offset:atOffset,count});
    if (!same) {shared.set(key,{packed,offset});parts.push(packed);offset+=bytes;}
  }
  if(parts.length===0) return undefined;
  const result=new Uint8Array(offset);
  let at=0;
  for(const part of parts) {result.set(part,at);at+=part.length;}
  return result;
}

function visitScalars(value:unknown,write:(value:unknown)=>void):void {
  if(Array.isArray(value)) for(const child of value) visitScalars(child,write);
  else write(value);
}

/** Buffer objects have a reserved element type and data array throughout scene JSON.
 * Traverse their containers, including inline geometry and nested child nodes, but
 * never traverse numeric data itself. Preserve unknown fields for loader validation. */
export function* bufferObjects(value:unknown, path='scene', seen=new WeakSet<object>()):Generator<[string,SceneBuffer]> {
  if (value && typeof value==='object') {if (seen.has(value)) return;seen.add(value);}
  if (Array.isArray(value)) {
    for (let i=0;i<value.length;i++) yield* bufferObjects(value[i],`${path}[${i}]`,seen);
  } else if (value && typeof value==='object') {
    const object=value as Record<string,unknown>;
    if (typeof object.type==='string' && Object.hasOwn(widths,object.type) && (Array.isArray(object.data) || typeof object.file==='string')) {
      yield [path,object as unknown as SceneBuffer];
    } else for (const [key,child] of Object.entries(object)) yield* bufferObjects(child,`${path}.${key}`,seen);
  }
}
