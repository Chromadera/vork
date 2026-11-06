import { writeScene, type Scene } from '../dist/index.js';
import { spawnSync } from 'node:child_process';
const asset = process.argv[2];
if (!asset) throw new Error('Usage: node examples/orbit.ts model.glb [output.mp4 | still-directory]');
const output = process.argv[3] ?? 'out/orbit.mp4';
const times = Array.from({length:49}, (_,i)=>i/48);
const positions = times.map(t=>[3*Math.cos(t*2*Math.PI),2,3*Math.sin(t*2*Math.PI)]);
const pitch = -Math.atan2(1.2,3);
// Pitch about X, then yaw about Y; Euler angles interpolate separately, so two keys orbit smoothly.
const rotation = (t:number) => ({euler:[pitch, Math.PI/2-t*2*Math.PI, 0] as [number,number,number]});
const scene = {
  version:2,
  nodes:[
    {type:'asset',name:'hero',file:asset},
    {type:'camera',name:'cam',material:{program:'std/camera',inputs:{lens:35,clip_end:30}}},
    {type:'light',name:'key',shape:'sun',energy:3,rotation:[-.38,-.35,-.16,.84]},
  ],
  render:{resolution:[640,360],duration:1},
  animation:[{tracks:[
    {path:['nodes','cam','translation'],keys:times.map((t,i)=>[t,positions[i]] as [number,number[]])},
    {path:['nodes','cam','rotation'],keys:[[0,rotation(0)],[1,rotation(1)]]},
  ]}],
} satisfies Scene;
const json = await writeScene(scene, output+'.scene.json');
const wrapper=process.env.VORK_WRAPPER,binary=process.env.VORK_BIN??'vork';
const result=spawnSync(wrapper??binary,[...(wrapper?[binary]:[]),'render',json,'--out',output,...(output.endsWith('.mp4')?[]:['--at','0.5'])],{stdio:'inherit'});
if(result.error)throw result.error;
if(result.status!==0)throw new Error(`vork exited ${result.status}`);
