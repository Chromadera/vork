// Both writers consume the same graph, derived from the Rust input schema.
import { readFileSync, writeFileSync } from 'node:fs';
const schema = JSON.parse(readFileSync(new URL('scene.schema.json', import.meta.url)));
function graph(schema) {
  if (!schema || typeof schema !== 'object') return {};
  const node = {};
  if (schema.$ref) node.ref = schema.$ref.split('/').at(-1);
  const branches = ['allOf', 'anyOf', 'oneOf'].flatMap(key => (schema[key] ?? []).map(graph));
  if (branches.length) node.branches = branches;
  if (schema.items) node.items = graph(schema.items);
  if (schema.additionalProperties && typeof schema.additionalProperties === 'object') node.values = graph(schema.additionalProperties);
  // Only schema-declared filename properties are paths; opaque input/table names
  // are visited through their value schema, never interpreted as field names.
  if (schema.properties) node.properties = Object.fromEntries(Object.entries(schema.properties).map(([key, value]) =>
    [key, ['file', 'source_origin'].includes(key) && [value.type].flat().includes('string') ? {file: true} : graph(value)]));
  return node;
}
const defs = Object.fromEntries(Object.entries(schema.$defs).map(([key, value]) => [key, graph(value)]));
const root = graph(schema);
// Remove branches that cannot lead to a file, including dead reference cycles.
const live = new Set();
function useful(node) {
  return node.file || (node.ref && live.has(node.ref)) || (node.branches ?? []).some(useful)
    || Object.values(node.properties ?? {}).some(useful) || (node.items && useful(node.items)) || (node.values && useful(node.values));
}
let count;
do { count = live.size; for (const [key, value] of Object.entries(defs)) if (useful(value)) live.add(key); } while (live.size !== count);
function prune(node) {
  if (!useful(node)) return undefined;
  return Object.fromEntries(Object.entries(node).flatMap(([key, value]) => {
    if (key === 'properties') return [[key, Object.fromEntries(Object.entries(value).filter(([, v]) => useful(v)).map(([k,v]) => [k, prune(v)]))]];
    if (key === 'branches') return [[key, value.filter(useful).map(prune)]];
    if (key === 'items' || key === 'values') return useful(value) ? [[key, prune(value)]] : [];
    return [[key, value]];
  }));
}
const result = {root: prune(root), defs: Object.fromEntries(Object.entries(defs).filter(([key]) => live.has(key)).map(([key,value]) => [key,prune(value)]))};
const json = JSON.stringify(result, null, 2) + '\n';
for (const [path, content] of [
  ['src/resource-slots.ts', '// Generated from the Rust scene schema; do not edit.\nexport const resourceSlots = ' + json.trim() + ' as const;\n'],
  ['../python/vork/_resource_slots.json', json],
]) {
  const url = new URL(path, import.meta.url);
  if (process.argv.includes('--check')) {
    if (readFileSync(url, 'utf8') !== content) throw new Error(`${path} is stale`);
  } else writeFileSync(url, content);
}
