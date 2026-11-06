import assert from 'node:assert/strict';
import {test} from 'node:test';
import * as sdk from '../dist/index.js';
import {checkOptions} from '../dist/lib/validate.js';

test('shape options suggest names and reject the misspelling', () => {
  for (const generate of [options => sdk.sphere('ball', options)]) {
    assert.throws(() => generate({raduis:2}), /unknown option "raduis"; did you mean "radius"\? Valid names:/);
    assert.throws(() => generate({zzzzzzzz:2}), error => !error.message.includes('did you mean') && /Valid names:/.test(error.message) && ['radius','rings','segments'].every(name => error.message.includes(name)));
  }
  assert.throws(() => sdk.extrude('plate',{outline:[],depht:2}), /did you mean "depth"/);
});

test('render and renderArguments diagnose options before invoking the CLI', () => {
  for (const render of [sdk.render, sdk.renderArguments]) {
    assert.throws(() => render('missing.json',{out:'out.png',proflie:[10,10]}), /unknown option "proflie"; did you mean "profile"\? Nearest valid names/);
    assert.throws(() => render('missing.json',{out:'out.png',zzzzzzzz:1}), error => /Nearest valid names/.test(error.message) && !error.message.includes('did you mean'));
  }
});

test('option diagnostics use transpositions, lexical ties and the CLI distance cutoff', () => {
  assert.throws(() => checkOptions({grb:1},['rgb']), /did you mean "rgb"/);
  assert.throws(() => checkOptions({cot:1},['cut','cat']), /did you mean "cat"\? Valid names: cat, cut/);
  assert.throws(() => checkOptions({zzzz:1},['radius']), /unknown option "zzzz"; Valid names: radius/);
  assert.throws(() => checkOptions({bad:1},[]), /unknown option "bad"; no valid names are declared/);
});
