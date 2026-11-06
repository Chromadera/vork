import test from 'node:test';
import assert from 'node:assert/strict';
import {scatter, grid} from '../dist/index.js';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
const transforms = result => {
 const {TRANSLATION,ROTATION}=result.instances.attributes;
 return result.buffers[TRANSLATION].data.map((p,i)=>{
  const [x,y,z,w]=result.buffers[ROTATION].data[i];
  return [1-2*(y*y+z*z),2*(x*y+w*z),2*(x*z-w*y),0,
   2*(x*y-w*z),1-2*(x*x+z*z),2*(y*z+w*x),0,
   2*(x*z+w*y),2*(y*z-w*x),1-2*(x*x+y*y),0,...p,1];
 });
};
test('scatter is seeded, samples surface area and aligns +Y to the face normal',()=>{
 const source={buffers:{p:{type:'vec3',data:[[0,0,0],[2,0,0],[0,0,1],[10,0,0],[16,0,0],[10,0,1]]}},geometry:{surface:{attributes:{POSITION:'p'}}}};
 const rows=transforms(scatter(source,1000,42));
 assert.deepEqual(rows,transforms(scatter(source,1000,42)));
 assert.notDeepEqual(rows,transforms(scatter(source,1000,43)));
 const larger=rows.filter(x=>x[12]>=10).length;
 assert.ok(larger>680&&larger<820,`area ratio 3:1: ${larger}`);
 for(const m of rows){
  assert.ok(m.every(Number.isFinite));assert.ok(m.slice(4,7).every((v,i)=>Math.abs(v-[0,-1,0][i])<1e-12));
  assert.equal(m[13],0);assert.ok(m[14]>=0&&m[14]<=1);
  const x=m[12]<10?m[12]/2:(m[12]-10)/6;assert.ok(x+m[14]<=1+1e-12);
 }
});
test('scatter rejects unreadable, degenerate and invalid geometry',()=>{
 assert.throws(()=>scatter(grid('p'),1,-1),/uint32/);
 assert.throws(()=>scatter(grid('p'),0,1),/positive/);
 const source=grid('p');source.buffers.p_position={type:'vec3',file:'positions.bin'};
 assert.throws(()=>scatter(source,1,1),/inline/);
 const invalid=grid('p');invalid.buffers.p_indices.data=[999,1,2];assert.throws(()=>scatter(invalid,1,1),/valid vertex/);
 const zero=grid('p');zero.buffers.p_position.data.fill([0,0,0]);assert.throws(()=>scatter(zero,1,1),/nonzero/);
});

test('generated CLI std dispatch reaches surface sampling',()=>{
 const script=readFileSync(new URL('../../../crates/vork-std/src/sdk.js',import.meta.url),'utf8');
 const evaluate=(name,args)=>JSON.parse(runInNewContext(script+'; JSON.stringify(stdEvaluate(name,args))',{name,args,structuredClone}));
 const source=grid('surface');
 assert.deepEqual(evaluate('scatter',{on_geometry:source,count:3,seed:8}),JSON.parse(JSON.stringify(scatter(source,3,8))));
 assert.ok(transforms(scatter(source,1,0)).length===1);
 const sides=Array.from({length:10},(_,seed)=>transforms(scatter(source,1,seed))[0][12]);
 assert.ok(Math.max(...sides)-Math.min(...sides)>0.5,'adjacent seeds spread the first sample');
 const other=grid('other');assert.notEqual(scatter(source,1,0).instances.attributes.TRANSLATION,scatter(other,1,0).instances.attributes.TRANSLATION);
});

test('scatter accepts inline position and index buffers',()=>{
 const source=grid('p');
 const expected=scatter(source,8,42);
 const geometry=source.geometry.p;
 geometry.attributes.POSITION=source.buffers[geometry.attributes.POSITION];
 geometry.indices=source.buffers[geometry.indices];
 assert.deepEqual(scatter(source,8,42),expected);
});
