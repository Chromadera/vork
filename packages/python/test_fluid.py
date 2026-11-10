"""Physical-unit and scheduling invariants, without GPU dependencies."""
import unittest
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
from vork.fluid import build

class BuilderTest(unittest.TestCase):
    def make(self, **changes):
        args=dict(callbacks='',extent=[3,1,3],particle_budget=1000,speed=3.)
        args.update(changes)
        return build(**args)

    def test_froude_scale_and_capillarity(self):
        a=self.make(surface_tension=0.)
        b=self.make(extent=[30,10,30],scale=10.,speed=3.*10**.5,surface_tension=0.)
        self.assertAlmostEqual(b['simulation']['dt']/a['simulation']['dt'],10**.5)
        # Physical capillarity transforms as L^-3, acceleration as L^-1.
        c=self.make(scale=10.)['programs']['init']['source']
        self.assertIn('GRAVITY = 0.981;',c)
        self.assertIn('CAPILLARY = 7.28e-08;',c)

    def test_order_and_budgets(self):
        a=self.make(whitewater_budget=57)
        self.assertEqual(len(a['buffers']['white_draw_position']['data']),57)
        self.assertNotIn('dims',a['buffers']['white_draw_position'])
        steps=a['simulation']['steps']
        self.assertLess(steps.index('project'),steps.index('marker_advect'))
        self.assertLess(steps.index('bin_fill'),steps.index('white_normals'))
        self.assertLess(steps.index('white_normals'),steps.index('white_step'))
        self.assertEqual(set(a['simulation']['state']),set(a['buffers'])-{'white_width'})
        for p in a['programs'].values():
            self.assertEqual(set(p['bindings'].values()),set(a['buffers'])-{'white_width'})

    def test_source_trials_are_independent_of_secondary_capacity(self):
        a=self.make(whitewater_budget=32)
        b=self.make(whitewater_budget=256)
        self.assertEqual(a['buffers']['white_birth_source']['dims'],b['buffers']['white_birth_source']['dims'])
        self.assertEqual(a['geometry']['whitewater']['ids'],'white_draw_id')
        self.assertEqual(a['simulation']['steps'][-1],'white_publish')
        self.assertLess(a['simulation']['steps'].index('white_step'),a['simulation']['steps'].index('white_birth'))

    def test_reject_unstable_and_malformed(self):
        for change in [dict(scale=0),dict(dt=1),dict(spacing=-1),dict(gravity=float('nan')),
                       dict(grid=[96,30,96]),dict(whitewater_budget=-1),dict(particle_budget=0)]:
            with self.subTest(change=change),self.assertRaises(ValueError): self.make(**change)

if __name__=='__main__': unittest.main()
