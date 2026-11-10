"""Authoring contracts for private fixed-clock cloth, without a GPU."""

import copy
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from vork.cloth import attach


def scene():
    return dict(
        nodes=[
            dict(
                type="mesh",
                name="fabric",
                geometry="mesh",
                materials=[dict(inputs=dict(base_color=[1, 0, 0]))],
            )
        ],
        buffers=dict(
            p=dict(type="vec3", data=[[0, 0, 0], [1, 0, 0], [0, 0, 1]]),
            uv=dict(type="vec2", data=[[0, 0], [1, 0], [0, 1]]),
        ),
        geometry=dict(mesh=dict(attributes=dict(POSITION="p", TEXCOORD_0="uv"))),
    )


class ClothTest(unittest.TestCase):
    def test_surface_and_clock_preserved(self):
        s = scene()
        s["simulation"] = dict(dt="1/120", t0=0, seed=42, state=[], init=[], steps=[])
        before = copy.deepcopy(s["nodes"][0]["materials"])
        attach(s, "fabric", pins=[0])
        self.assertEqual(s["simulation"]["dt"], "1/120")
        self.assertEqual(s["simulation"]["seed"], 42)
        self.assertEqual(s["nodes"][0]["materials"], before)
        g = s["geometry"][s["nodes"][0]["geometry"]]
        self.assertEqual(g["attributes"]["TEXCOORD_0"], "uv")
        self.assertNotEqual(g["attributes"]["POSITION"], "p")
        self.assertTrue(all("dims" not in b for b in s["buffers"].values()))
        self.assertTrue(
            all(
                '#include "cloth/solver.slang"' in p["source"]
                for p in s["programs"].values()
            )
        )

    def test_rest_positions_and_named_capacity_receipt(self):
        s = scene()
        positions = copy.deepcopy(s["buffers"]["p"]["data"])
        attach(s, "fabric", collision_margin=0.006)
        g = s["geometry"][s["nodes"][0]["geometry"]]
        status = g["status"]
        self.assertEqual(s["buffers"][status["buffer"]], dict(type="u32", data=[0]))
        self.assertIn("fabric", status["message"])
        self.assertIn("512", status["message"])
        self.assertIn("collision_margin", status["message"])
        rest = next(v for k, v in s["buffers"].items() if k.endswith("rest_position"))
        self.assertEqual(rest["data"], positions)
        self.assertTrue(all("contact_status" in p["bindings"] for p in s["programs"].values()))

    def test_authored_predecessor_normals_survive(self):
        s = scene()
        normals = [[0, -1, 0]] * 3
        s["buffers"]["artist_normal"] = dict(type="vec3", data=normals)
        s["geometry"]["mesh"]["attributes"]["NORMAL"] = "artist_normal"
        attach(s, "fabric")
        g = s["geometry"][s["nodes"][0]["geometry"]]
        self.assertEqual(s["buffers"][g["attributes"]["NORMAL"]]["data"], normals)

    def test_invalid_attachment_is_transactional(self):
        for kwargs in [
            dict(dt=0),
            dict(iterations=3),
            dict(density=0),
            dict(collision_margin=-1),
            dict(pins=[3]),
            dict(collider="fabric"),
        ]:
            s = scene()
            original = copy.deepcopy(s)
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                attach(s, "fabric", **kwargs)
            self.assertEqual(s, original)

    def test_duplicate_faces_are_rejected(self):
        s = scene()
        s["buffers"]["index"] = dict(type="u32", data=[0, 1, 2, 2, 1, 0])
        s["geometry"]["mesh"]["indices"] = "index"
        with self.assertRaisesRegex(ValueError, "remove duplicate"):
            attach(s, "fabric")

    def test_clock_and_frame_errors_name_fix(self):
        s = scene()
        s["simulation"] = dict(dt="1/120")
        with self.assertRaisesRegex(ValueError, "share scene.simulation.dt"):
            attach(s, "fabric", dt="1/240")
        s = scene()
        s["nodes"][0]["translation"] = [1, 0, 0]
        with self.assertRaisesRegex(ValueError, "bake translation"):
            attach(s, "fabric")
        s = scene()
        attach(s, "fabric")
        with self.assertRaisesRegex(ValueError, "already attached|in-memory"):
            attach(s, "fabric")


if __name__ == "__main__":
    unittest.main()
