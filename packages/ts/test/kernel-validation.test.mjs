import {existsSync} from 'node:fs';
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {test} from 'node:test';
import {fileURLToPath} from 'node:url';
import {mkdtemp,readFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import * as sdk from '../dist/index.js';

const root = fileURLToPath(new URL('../../..', import.meta.url));
const validate = scenes => spawnSync('cargo', ['run','--quiet','-p','vork-scene','--example','validate-scenes'], {
  cwd:root, input:JSON.stringify(scenes), encoding:'utf8',
});
const mesh = geometry => ({version:2,...geometry,nodes:[{type:'camera',name:'camera'},{type:'mesh',geometry:Object.keys(geometry.geometry)[0]}]});

test('SDK geometry engines emit fragments accepted by the Rust kernel', async () => {
  const fixtures=[];
  for (const shape of ['box','sphere','cylinder','cone','torus','capsule','roundedBox','grid'])
    fixtures.push(mesh(sdk[shape](shape)));
  fixtures.push(mesh(sdk.revolve('revolve',{profile:[[0,0],[1,0],[0.5,1],[0,1.2]]})));
  const geometry=sdk.grid('surface'), instances=sdk.scatter(geometry,3,7);
  fixtures.push({version:2,...geometry,buffers:{...geometry.buffers,...instances.buffers},nodes:[{type:'camera',name:'camera'},{type:'mesh',geometry:'surface',instances:instances.instances}]});
  const previousBinary=process.env.VORK_BIN;
  process.env.VORK_BIN=previousBinary ?? resolve(root,process.env.CARGO_TARGET_DIR ?? 'target','release','vork');
  const font = join(root,'crates/vork-std/tests/fonts/LiberationSans-Regular.ttf');
  try {
    for (const [builder, token, options] of [
      [sdk.extrude, 'extrude', {outline:[[0,0],[1,0],[0,1]],bevelDepth:0.02,bevelResolution:4}],
      [sdk.text, 'text', {body:'x',font:join(root,'crates/vork-std/tests/fonts/LiberationSans-Regular.ttf')}],
    ]) {
      if (token === 'text' && !existsSync(font)) continue; // private fixture does not ship
      const fragment = builder(undefined, options);
      assert.deepEqual(fragment, builder(token, options));
      assert.equal(fragment.geometry[token].attributes.POSITION, token + '_position');
      fixtures.push(mesh(fragment));
    }
    fixtures.push(mesh(sdk.extrude('plate',{outline:[[0,0],[2,0],[2,1],[0,1]],holes:[[[0.5,0.25],[1.5,0.25],[1,0.75]]],depth:0.1})));
    if (existsSync(font)) fixtures.push(mesh(sdk.text('label',{body:'vork',font:join(root,'crates/vork-std/tests/fonts/LiberationSans-Regular.ttf')})));
  } finally {
    if (previousBinary === undefined) delete process.env.VORK_BIN;
    else process.env.VORK_BIN=previousBinary;
  }
  const directory=await mkdtemp(join(tmpdir(),'vork-kernel-sdk-'));
  try {
    const destination=await sdk.writeScene(fixtures[0],join(directory,'scene.json'));
    fixtures.push(JSON.parse(await readFile(destination,'utf8')));
  } finally {await rm(directory,{recursive:true,force:true});}
  const result=validate(fixtures);
  assert.equal(result.status,0,result.stderr);
});

test('CLI-backed std helpers raise with the child diagnostic', () => {
  const previousBinary=process.env.VORK_BIN;
  process.env.VORK_BIN=process.execPath; // `node std text -` fails with a stderr message
  try {
    assert.throws(() => sdk.text('label',{body:'x',font:'missing.ttf'}), /^Error: vork std text: .*Cannot find module/s);
  } finally {
    if (previousBinary === undefined) delete process.env.VORK_BIN;
    else process.env.VORK_BIN=previousBinary;
  }
});
