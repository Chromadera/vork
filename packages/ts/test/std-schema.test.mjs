import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';
import Ajv2020 from 'ajv/dist/2020.js';

const schema = JSON.parse(readFileSync(new URL('../scene.schema.json', import.meta.url)));
const ajv = new Ajv2020({strict:false, validateFormats:false});
const validate = ajv.compile(schema);
const scene = geometry => ({version:2, geometry:{g:geometry}, nodes:[{type:'camera'},{type:'mesh',geometry:'g'}]});

test('editor schema accepts buffer geometry and authoring builders', () => {
  assert.equal(validate(scene({attributes:{POSITION:{type:'vec3',data:[[0,0,0],[1,0,0],[0,1,0]]}}})),true);
  for (const std of ['sphere','text','box','revolve','extrude'])
    assert.equal(validate(scene({std})),true);
});

test('editor schema validates the bundled starter', () => {
  const starter = JSON.parse(readFileSync(new URL('../../../crates/vork-scene/src/starter.json', import.meta.url)));
  assert.equal(validate(starter), true, JSON.stringify(validate.errors));
  assert.equal(validate(scene({std:42})), false);
  assert.equal(validate(scene({std:'sphere',name:'forbidden'})), false);
});

test('editor schema validates representative public examples', () => {
  for (const name of ['first-scene', 'black-hole/scene', 'water/scene',
    ...['fog-glow', 'ghosts', 'simple-star', 'streaks'].map(name => `compositor-effects/glare-${name}`)]) {
    const value = JSON.parse(readFileSync(new URL(`../../../examples/${name}.json`, import.meta.url)));
    assert.equal(validate(value), true, `${name}: ${JSON.stringify(validate.errors)}`);
  }
});
