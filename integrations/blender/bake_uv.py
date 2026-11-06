# SPDX-License-Identifier: GPL-3.0-or-later
"""Qualify a single-image UV bake: finite, nondegenerate, nonoverlapping tile 0."""
import math,sys
from fractions import Fraction
from .materials import UnsupportedMaterial


def orientation(a,b,c):
    if c==a or c==b:
        return 0
    left=(b[0]-a[0])*(c[1]-a[1]);right=(b[1]-a[1])*(c[0]-a[0])
    value=left-right
    # Bound double arithmetic roundoff, not a UV-area tolerance. Resolve
    # uncertain signs exactly from the source binary floating-point values.
    if abs(value)>8*sys.float_info.epsilon*(abs(left)+abs(right)):
        return 1 if value>0 else -1
    a,b,c=[tuple(Fraction(v) for v in p) for p in (a,b,c)]
    value=(b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])
    return (value>0)-(value<0)


def overlap(a,b):
    # A separating edge includes touching boundaries: only shared interior
    # area is ambiguous. Ordinary adjacent UV triangles remain qualified.
    for first,second in ((a,b),(b,a)):
        for i in range(3):
            if all(orientation(first[i],first[(i+1)%3],p)<=0 for p in second):
                return False
    return True


def qualify(triangles):
    axis=max(1,math.isqrt(len(triangles)))
    cells={};accepted=[]
    for source in triangles:
        triangle=tuple(tuple(float(v) for v in p) for p in source)
        if any(not math.isfinite(v) or v<0 or v>1 for p in triangle for v in p):
            raise UnsupportedMaterial('value bake requires UVs inside the single [0,1] image tile')
        sign=orientation(*triangle)
        if sign==0:
            raise UnsupportedMaterial('value bake cannot represent a degenerate UV triangle')
        if sign<0:triangle=(triangle[0],triangle[2],triangle[1])
        low=[min(p[c] for p in triangle) for c in (0,1)];high=[max(p[c] for p in triangle) for c in (0,1)]
        bounds=[(min(axis-1,int(low[c]*axis)),min(axis-1,int(high[c]*axis))) for c in (0,1)]
        keys=[(x,y) for x in range(bounds[0][0],bounds[0][1]+1) for y in range(bounds[1][0],bounds[1][1]+1)]
        candidates=set(i for key in keys for i in cells.get(key,()))
        for i in candidates:
            other,other_low,other_high=accepted[i]
            if any(high[c]<=other_low[c] or other_high[c]<=low[c] for c in (0,1)):
                continue
            if overlap(triangle,other):
                raise UnsupportedMaterial('value bake cannot represent overlapping UV triangle interiors')
        index=len(accepted);accepted.append((triangle,low,high))
        for key in keys:cells.setdefault(key,[]).append(index)


def qualify_mesh(mesh,uv):
    mesh.calc_loop_triangles()
    qualify([tuple(tuple(uv.data[i].uv) for i in triangle.loops) for triangle in mesh.loop_triangles])
