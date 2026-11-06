#!/usr/bin/env python3
"""Independent f64 oracle: weak deflection, critical capture, RK4 convergence/cost."""
import json
import math
import time
import unittest

def step(u,v,h):
    def f(u,v): return v,-u+3*u*u
    a=f(u,v);b=f(u+h*a[0]/2,v+h*a[1]/2)
    c=f(u+h*b[0]/2,v+h*b[1]/2);d=f(u+h*c[0],v+h*c[1])
    return u+h*(a[0]+2*b[0]+2*c[0]+d[0])/6,v+h*(a[1]+2*b[1]+2*c[1]+d[1])/6

def trace(b,h=.0025):
    # Incoming asymptote u=0, du/dphi=1/b; event interpolation at escape.
    u,v=0.,1/b
    for n in range(200000):
        un,vn=step(u,v,h)
        if un>=.5: return 'capture',(n+1)*h,n+1
        if un<0: return 'escape',(n+u/(u-un))*h,n+1
        u,v=un,vn
    return 'exhausted',0,200000


# CPU transcription of the authored polygonal step, checked against the independent orbit equation.
def polygon_trace(b,step):
 p=[b,0.,-1000.];d=[0.,0.,1.];previous=0.
 for n in range(100000):
  r=math.sqrt(sum(x*x for x in p))
  if r<=.601:return 'capture',0,n
  if p[2]>1000 or r>1500:return 'escape',math.atan2(-d[0],d[2]),n
  u=.5/r;index=(1+u)**3/(1-u);g=-1.5/(r*(r+.5))-.5/(r*(r-.5));t=min(1,max(0,(r-500)/500));taper=1-t*t*(3-2*t);derivative=-6*t*(1-t)/500
  g=(index*g*taper+(index-1)*derivative)/(1+(index-1)*taper)
  gradient=[g*x/r for x in p];dot=sum(a*b for a,b in zip(gradient,d));h=min(step*r,max(r-.6,.0001)*.25)
  mid=d[:]
  for _ in range(3):
   dot=sum(a*b for a,b in zip(gradient,mid));mid=[a+.5*previous*(b-c*dot) for a,b,c in zip(d,gradient,mid)];ml=math.sqrt(sum(x*x for x in mid));mid=[x/ml for x in mid]
  dot=sum(a*b for a,b in zip(gradient,mid))
  d=[a+.5*(h+previous)*(b-c*dot) for a,b,c in zip(d,gradient,mid)];previous=h;length=math.sqrt(sum(x*x for x in d));d=[x/length for x in d];p=[a+h*b for a,b in zip(p,d)]
 return 'exhausted',0,n

class Geodesics(unittest.TestCase):
    def test_weak_deflection(self):
        for b in [100.,200.,400.]:
            kind,angle,_=trace(b)
            self.assertEqual(kind,'escape')
            # Analytic asymptotic expansion through second order.
            expected=4/b+15*math.pi/(4*b*b)
            self.assertLess(abs(angle-math.pi-expected),50/b**3)
    def test_photon_sphere(self):
        u,v=1/3,0.
        for _ in range(1000):u,v=step(u,v,.01)
        self.assertAlmostEqual(u,1/3,places=10)
        self.assertAlmostEqual(1/u,3.,places=9)
        critical=3*math.sqrt(3)
        self.assertEqual(trace(critical*(1-1e-4))[0],'capture')
        self.assertEqual(trace(critical*(1+1e-4))[0],'escape')
    def test_convergence(self):
        for b in [5.2,5.3,6.,20.]:
            values=[trace(b,h) for h in [.01,.005,.0025]]
            self.assertTrue(all(v[0]=='escape' for v in values))
            self.assertLess(abs(values[0][1]-values[2][1]),2e-5)
    def test_finite_observer_invariant(self):
        r=12.;alpha=.4;f=1-2/r;b=r*math.sin(alpha)/math.sqrt(f)
        u=1/r;v=math.cos(alpha)/b
        self.assertAlmostEqual(v*v+u*u-2*u**3,1/b**2,places=14)
        for _ in range(1000):
            u,v=step(u,v,.001)
            self.assertLess(abs(v*v+u*u-2*u**3-1/b**2),1e-12)

    def test_authored_step_converges(self):
        for b in [5.3,6.,10.,20.,50.]:
            coarse=polygon_trace(b,.02);fine=polygon_trace(b,.01);reference=polygon_trace(b,.0025)
            self.assertEqual(coarse[0],'escape')
            self.assertLess(abs(fine[1]-reference[1]),abs(coarse[1]-reference[1])*.4)
            exact=trace(b)[1]-math.pi
            difference=math.atan2(math.sin(coarse[1]-exact),math.cos(coarse[1]-exact))
            self.assertLess(abs(difference),.001)
        self.assertEqual(polygon_trace(5.,.02)[0],'capture')

if __name__=='__main__':
    result=unittest.main(exit=False)
    if not result.result.wasSuccessful(): raise SystemExit(1)
    start=time.perf_counter()
    counts=[trace(3+i*.03,.01)[2] for i in range(1000)]
    elapsed=time.perf_counter()-start
    print(json.dumps({'oracle_seconds':elapsed,'rays':len(counts),'mean_steps':sum(counts)/len(counts),'max_steps':max(counts),'ns_per_f64_step':elapsed*1e9/sum(counts)},indent=2))
