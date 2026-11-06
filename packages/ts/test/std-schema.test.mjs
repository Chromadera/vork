import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {test} from 'node:test';
import Ajv2020 from 'ajv/dist/2020.js';

const schema = JSON.parse(readFileSync(new URL('../scene.schema.json', import.meta.url)));
const ajv = new Ajv2020({strict:false, validateFormats:false});
const validate = ajv.compile(schema);
const scene = geometry => ({version:2, geometry:{g:geometry}, nodes:[{type:'camera'},{type:'mesh',geometry:'g'}]});

test('kernel schema accepts buffer geometry and rejects builders', () => {
  assert.equal(validate(scene({attributes:{POSITION:{type:'vec3',data:[[0,0,0],[1,0,0],[0,1,0]]}}})),true);
  for (const std of ['sphere','text','box','revolve','extrude'])
    assert.equal(validate(scene({std,attributes:{}})),false);
});
