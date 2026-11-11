"""Physical-unit and scheduling invariants, without GPU dependencies."""
import inspect
import re
import unittest
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
from vork import fluid
from vork.fluid import build

class BuilderTest(unittest.TestCase):
    def make(self, **changes):
        args=dict(callbacks='',extent=[3,1,3],particle_budget=1000,speed=3.)
        args.update(changes)
        return build(**args)

    def test_documented_output_keys_match_builder(self):
        help_text = (Path(__file__).resolve().parents[2] /
                     'libraries/fluid/HELP.txt').read_text()
        documents = {'help': help_text, 'module': fluid.__doc__,
                     'build': inspect.getdoc(build)}
        for label, document in documents.items():
            keys = re.findall(r'(geometry|buffers)\["([^"\n]+)"\]', document)
            with self.subTest(document=label):
                self.assertTrue(keys)
                scene = self.make(whitewater_budget=8)
                for table, key in keys:
                    self.assertIn(key, scene[table], (label, table, key))
                self.assertEqual({key for table, key in keys if table == 'geometry'},
                                 set(scene['geometry']))
                # The default topic is the first contract an author sees.
                if label == 'help':
                    self.assertIn('geometry["water"]', document.split('<!-- all -->')[0])
        for budget in (0, 8):
            with self.subTest(whitewater_budget=budget):
                scene = self.make(whitewater_budget=budget)
                self.assertEqual(set(scene['geometry']),
                                 {'water', 'floor_film'} | ({'whitewater'} if budget else set()))
                for geometry in scene['geometry'].values():
                    references = list(geometry['attributes'].values())
                    references += [geometry[k] for k in ('widths', 'count', 'ids')
                                   if k in geometry]
                    for key in references:
                        self.assertIn(key, scene['buffers'])
                self.assertEqual(any(k.startswith('white_') for k in scene['buffers']),
                                 bool(budget))

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

    def rectangular(self, **changes):
        args = dict(extent=(.24, .16, .16), origin=[1., -.2, 3.],
                    grid=[24, 16, 16], surface_grid=(72, 48, 48),
                    bins=[12, 8, 8], film_grid=(48, 1, 32),
                    particle_budget=[7, 3, 5])
        args.update(changes)
        return self.make(**args)

    def test_rectangular_dimension_family(self):
        scene = self.rectangular()
        buffers = scene['buffers']
        for name, dims in [('phi', [24, 16, 16]), ('surface', [72, 48, 48]),
                           ('bins', [12, 8, 8]), ('wetness', [48, 1, 32]),
                           ('film_surface', [48, 3, 32]), ('markers', [7, 3, 5])]:
            self.assertEqual(buffers[name]['dims'], dims)
        self.assertEqual(scene['programs']['marker_init']['dispatch'], 105)
        count = self.rectangular(particle_budget=19)
        self.assertEqual(count['buffers']['markers']['dims'], [19, 1, 1])
        derived = self.rectangular(surface_grid=None, bins=None, film_grid=None)
        self.assertEqual(derived['buffers']['surface']['dims'], [48, 32, 32])
        self.assertEqual(derived['buffers']['bins']['dims'], [24, 16, 16])
        self.assertEqual(derived['buffers']['wetness']['dims'], [24, 1, 16])

    def test_dimension_rank_and_integer_boundaries(self):
        malformed = [(4, 4), (4, 4, 4, 4)]
        for invalid in (0, -1, 4., True):
            for axis in range(3):
                dims = [4, 4, 4]
                dims[axis] = invalid
                malformed.append(dims)
        for name in ('grid', 'surface_grid', 'bins', 'film_grid', 'particle_budget'):
            for dims in malformed:
                with self.subTest(argument=name, dims=dims), self.assertRaises(ValueError):
                    self.rectangular(**{name: dims})

    def test_optional_dimensions_only_none_selects_defaults(self):
        derived = self.rectangular(surface_grid=None, bins=None, film_grid=None)
        explicit = self.rectangular(surface_grid=[48, 32, 32], bins=[24, 16, 16],
                                    film_grid=[24, 1, 16])
        self.assertEqual(derived, explicit)
        for name in ('surface_grid', 'bins', 'film_grid'):
            for dims in ([], (), 0, False, ''):
                with self.subTest(argument=name, dims=dims):
                    # Wrong scalar types fail sequence conversion; rank-zero
                    # sequences fail the existing dimension validator.
                    with self.assertRaises((TypeError, ValueError)):
                        self.rectangular(**{name: dims})

    def test_physical_dimension_boundaries(self):
        for name in ('grid', 'surface_grid', 'bins'):
            with self.subTest(argument=name), self.assertRaises(ValueError):
                self.rectangular(**{name: (16, 16, 16)})
        for dims in ((24, 2, 16), (24, 1, 24)):
            with self.subTest(film_grid=dims), self.assertRaises(ValueError):
                self.rectangular(film_grid=dims,
                                 callbacks='bool film_accept(float2 xz) { return false; }')
        for extent in ((.24, .16), (.24, .16, .16, .16), (0, .16, .16),
                       (-.24, .16, .16), (float('nan'), .16, .16),
                       (.24, float('inf'), .16)):
            with self.subTest(extent=extent), self.assertRaises(ValueError):
                self.rectangular(extent=extent)
        for origin in ((0, 0), (0, 0, 0, 0), (0, float('nan'), 0),
                       (float('inf'), 0, 0)):
            with self.subTest(origin=origin), self.assertRaises(ValueError):
                self.rectangular(origin=origin)

    def test_reject_unstable_and_malformed(self):
        for change in [dict(scale=0),dict(dt=1),dict(spacing=-1),dict(gravity=float('nan')),
                       dict(grid=[96,30,96]),dict(whitewater_budget=-1),dict(particle_budget=0)]:
            with self.subTest(change=change),self.assertRaises(ValueError): self.make(**change)

if __name__=='__main__': unittest.main()
