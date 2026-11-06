import { mkdir, open, rename, rm, realpath, lstat } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import { dirname, resolve, relative, basename } from 'node:path';
import type { Scene } from './schema.js';
import { externalBuffers } from './lib/buffers.js';
export type * from './schema.js';

async function fileIdentity(path: string): Promise<string> {
  try { return await realpath(path); }
  catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
    const parent = dirname(path);
    return parent === path ? path : resolve(await fileIdentity(parent), basename(path));
  }
}

/** Write external resource paths relative to the scene directory, resolving inputs against baseDir. */
async function withAssetPaths(value: Scene, baseDir: string, sceneDir: string): Promise<Scene> {
  const assetPath = async (file: string): Promise<string> => {
    const source = resolve(baseDir, file);
    return relative(sceneDir, resolve(await fileIdentity(dirname(source)), basename(source)));
  };
  const result = structuredClone(value);
  // One declaration of resource-bearing fields. `*` means array items or
  // opaque table values, never a resource name interpreted as a field.
  const edges: Record<string, [string, string][]> = {
    scene: [['nodes.*','node'],['render','render'],['animation.*','layer'],
      ...['images','buffers','programs','geometry','materials'].map(kind => [`${kind}.*`,kind] as [string,string])],
    node: [['children.*','node'],['geometry','geometry'],['materials.*','materials'],
      ['instances.count','buffers'],['instances.attributes.*','buffers'],['skin.inverse_bind_matrices','buffers'],
      ['image','images'],['material','materials']],
    geometry: [['indices','buffers'],['curve_vertex_counts','buffers'],['widths','buffers'],
      ['attributes.*','buffers'],['morphs.*.POSITION','buffers'],['morphs.*.NORMAL','buffers']],
    materials: [['program','programs'],['inputs.*','input']],
    input: [['image','images'],['grid','buffers']],
    images: [['render','render']],
    render: [['compositor.*','materials'],['compositor_per_step.*','materials'],['display','materials']],
    programs: [['bindings.*','buffers']],
    layer: [['tracks.*.keys.times','buffers'],['tracks.*.keys.values','buffers']],
  };
  const files: Record<string, string[]> = {
    buffers:['file'], images:['file'], programs:['file','source_origin'],
    node:['file'], layer:['clip.file'],
  };
  type ObjectValue = Record<string, any>;
  const slots = async (value: any, path: string[], visit: (parent: ObjectValue, key: string) => Promise<void>): Promise<void> => {
    if (!value || typeof value !== 'object') return;
    const head = path[0]!;
    const tail = path.slice(1);
    for (const key of head === '*' ? Object.keys(value) : [head]) {
      if (tail.length) await slots(value[key],tail,visit);
      else await visit(value,key);
    }
  };
  const seen = new Set<object>();
  const resource = async (kind: string, value: any): Promise<void> => {
    if (!value || typeof value !== 'object' || Array.isArray(value) || seen.has(value)) return;
    seen.add(value);
    for (const path of files[kind] ?? []) await slots(value,path.split('.'),async (parent,key) => {
      if (typeof parent[key] === 'string') parent[key] = await assetPath(parent[key]);
    });
    for (const [path,target] of edges[kind] ?? []) await slots(value,path.split('.'),async (parent,key) => {
      await resource(target,parent[key]);
    });
  };
  await resource('scene',result);
  return result;
}
/** Write scene JSON with relative resource paths and a binary sidecar for large buffers. */
export async function writeScene(value: Scene, path: string, baseDir = process.cwd()): Promise<string> {
  const destination = resolve(path);
  await mkdir(dirname(destination), { recursive: true });
  const destinationInfo = await lstat(destination).catch((error: NodeJS.ErrnoException) => {
    if (error.code !== 'ENOENT') throw error;
    return undefined;
  });
  if (destinationInfo?.isSymbolicLink()) throw new Error('writeScene destination must not be a symbolic link');
  const sceneDir = await realpath(dirname(destination));
  const physicalDestination = resolve(sceneDir, basename(destination));
  const scene = await withAssetPaths(value, resolve(baseDir), sceneDir);
  // An immutable sidecar plus one JSON rename is the publication boundary.
  // Keep older sidecars: a concurrent reader may still hold the previous JSON.
  const binaryPath = `${physicalDestination}.buffers.${randomUUID()}.bin`;
  const binary = externalBuffers(scene, relative(sceneDir, binaryPath));
  const text = JSON.stringify(scene, null, 2) + '\n';
  const temporary = `${physicalDestination}.${randomUUID()}.tmp`;
  let temporaryCreated = false;
  try {
    if (binary) {
      const file = await open(binaryPath, 'wx', 0o600);
      try { await file.writeFile(binary); } finally { await file.close(); }
    }
    const file = await open(temporary, 'wx', 0o600);
    temporaryCreated = true;
    try {
      await file.writeFile(text);
      if (destinationInfo?.isFile()) await file.chmod(destinationInfo.mode & 0o777);
    } finally { await file.close(); }
    await rename(temporary, physicalDestination);
  } catch (error) {
    if (temporaryCreated) await rm(temporary, {force:true});
    throw error;
  }
  return destination;
}

export { box, sphere, cylinder, cone, torus, capsule, roundedBox, grid, revolve } from './lib/shapes.js';
export type { GeometryFragment } from './lib/shapes.js';
export { scatter, type InstanceFragment } from './lib/scatter.js';
export { text, type TextOptions } from './lib/text.js';
export { extrude, type ExtrudeOptions } from './lib/extrude.js';
export { render, renderArguments, type RenderOptions, type RenderResult, type ExposureSummary } from './lib/render.js';
export { environmentImage, sky, type EnvironmentImageOptions, type EnvironmentFragment } from './lib/environment.js';
