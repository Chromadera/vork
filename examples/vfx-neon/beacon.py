"""Original After Rain light sculpture. Generated geometry is CC0.

No downloaded lettering, model, texture, or franchise reference.
"""
import json, math
from pathlib import Path
import numpy as np

SCALE=.8
HUB_OFFSETS=[[-.26,0,-.26],[-.26,0,.26],[.26,0,-.26],[.26,0,.26]]

# Deliberately simple single-stroke letterforms, drawn for this sculpture.
LETTERS={
 'A':[[(0,0),(.5,1),(1,0)],[(.23,.45),(.77,.45)]],
 'F':[[(0,0),(0,1),(1,1)],[(0,.52),(.8,.52)]],
 'T':[[(0,1),(1,1)],[(.5,1),(.5,0)]],
 'E':[[(1,0),(0,0),(0,1),(1,1)],[(0,.5),(.8,.5)]],
 'R':[[(0,0),(0,1),(.8,1),(1,.8),(1,.62),(.8,.5),(0,.5)],[(.5,.5),(1,0)]],
 'I':[[(.5,0),(.5,1)]],
 'N':[[(0,0),(0,1),(1,0),(1,1)]],
}

def geometry(wall_x):
 anchor=(wall_x+1.2)/SCALE
 groups={n:[] for n in ['cyan','amber','metal','orbit']}
 def line(pts,c='cyan',r=.012):groups[c].append((np.array(pts,float),r))
 for j in range(9):
  pts=[]
  for t in np.linspace(0,math.pi,100):
   a=j*math.tau/9+t*1.15;rad=.92*math.sin(t);pts.append([rad*math.cos(a),1.25*math.cos(t),rad*math.sin(a)])
  line(pts,'orbit',.014)
 for k in range(3):
  t=np.linspace(0,math.tau,120);line([[.26*math.cos(a),.38*math.sin(a),.16*math.sin(a*2+k)] for a in t],'amber',.018)
 # Wall arm and two triangular braces tie the seed to the masonry.
 line([[0,1.26,0],[0,1.65,0],[anchor,.4,-2.5]],'metal',.035)
 line([[anchor,.4,-2.75],[.5,1.65,0],[anchor,.4,-2.25]],'metal',.025)
 # Original letterforms on a projecting blade, behind the sculptural volume.
 for word,y in [('AFTER',.3),('RAIN',-.05)]:
  for k,ch in enumerate(word):
   for stroke in LETTERS[ch]:line([[4.2+(k*1.4+x)*.15,y+v*.23,-3.5] for x,v in stroke],'cyan',.014)
 line([[4.12,-.15,-3.5],[4.12,.64,-3.5],[anchor,.64,-3.5]],'amber',.012)
 return groups
def write(path,wall_x):
    """Write ordinary scene buffers; each tube has analytic smooth normals."""
    result={}
    for name,lines in geometry(wall_x).items():
        positions=[];normals=[];indices=[]
        for points,radius in lines:
            tangents=np.gradient(points,axis=0)
            tangents/=np.linalg.norm(tangents,axis=1)[:,None]
            _,_,basis=np.linalg.svd(points-points.mean(axis=0),full_matrices=True)
            axis=basis[-1]
            u=np.cross(tangents,axis);u/=np.linalg.norm(u,axis=1)[:,None]
            v=np.cross(tangents,u);base=len(positions);sides=8
            for p,a,b in zip(points,u,v):
                for j in range(sides):
                    n=a*math.cos(j*math.tau/sides)+b*math.sin(j*math.tau/sides)
                    positions.append((p+radius*n).tolist());normals.append(n.tolist())
            for k in range(len(points)-1):
                for j in range(sides):
                    a=base+k*sides+j;b=base+k*sides+(j+1)%sides;c=a+sides;d=b+sides
                    indices.extend([a,b,c,b,d,c])
        if name=='metal':
            # Four dark circular fittings around the core; these are not ground contacts.
            for x,_,z in HUB_OFFSETS:
                for y,normal in [(0,[0,-1,0]),(.045,[0,1,0])]:
                    start=len(positions)
                    positions.append([x,y,z]);normals.append(normal)
                    for j in range(33):
                        angle=j*math.tau/32
                        positions.append([x+.055*math.cos(angle),y,z+.055*math.sin(angle)]);normals.append(normal)
                    for j in range(32):
                        tri=[start,start+j+1,start+j+2]
                        indices.extend(tri if y==0 else tri[::-1])
                start=len(positions)
                for j in range(33):
                    angle=j*math.tau/32;n=[math.cos(angle),0,math.sin(angle)]
                    for y in [0,.045]:positions.append([x+.055*n[0],y,z+.055*n[2]]);normals.append(n)
                for j in range(32):
                    a=start+2*j;indices.extend([a,a+1,a+2,a+2,a+1,a+3])
        result[name]={'positions':positions,'normals':normals,'indices':indices}
    Path(path).write_text(json.dumps(result,separators=(',',':'))+'\n')

if __name__=='__main__':
    import sys
    write(sys.argv[1],float(sys.argv[2]))
