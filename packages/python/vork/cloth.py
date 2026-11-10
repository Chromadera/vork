"""GPU XPBD cloth authored as ordinary compute programs and private geometry state.

Use attach() on a native mesh before write_scene(). See vork help cloth --all.
"""

import math
import hashlib
from copy import deepcopy
from fractions import Fraction

__all__ = ["attach"]


def sub(a, b):
    return tuple(x - y for x, y in zip(a, b))


def dot(a, b):
    return sum(x * y for x, y in zip(a, b))


def cross(a, b):
    return (
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    )


def norm(a):
    return math.sqrt(dot(a, a))


def colored(items):
    groups = []
    used = []
    for item in items:
        ids = set(item[0])
        k = next((k for k, s in enumerate(used) if not ids & s), len(used))
        if k == len(used):
            used.append(set())
            groups.append([])
        used[k].update(ids)
        groups[k].append(item)
    offsets = [0]
    ordered = []
    for group in groups:
        ordered += group
        offsets.append(len(ordered))
    return ordered, offsets


def _build(
    *,
    positions,
    triangles,
    pins=(),
    density=0.2,
    stretch_modulus=50000.0,
    bending_rigidity=0.002,
    damping=0.8,
    iterations=64,
    dt="1/240",
    force="return float3(0,-9.81,0);",
    contact=None,
    body=None,
    thickness=0.004,
    uvs=None,
):
    if not math.isfinite(thickness) or thickness <= 0:
        raise ValueError("cloth collision_margin must be positive and finite")
    n = len(positions)
    if not 3 <= n <= 1024:
        raise ValueError(
            "cloth requires 3..1024 vertices in one workgroup; split larger meshes into independent patches"
        )
    if any(len(p) != 3 or not all(math.isfinite(x) for x in p) for p in positions):
        raise ValueError("positions must be finite xyz triples")
    if len(triangles) % 3:
        raise ValueError("triangles needs a multiple of three indices")
    if any(type(i) is not int or not 0 <= i < n for i in triangles):
        raise ValueError("triangle index out of range")
    if (
        not all(
            math.isfinite(x) and x >= 0
            for x in [stretch_modulus, bending_rigidity, damping]
        )
        or not math.isfinite(density)
        or density <= 0
        or stretch_modulus <= 0
    ):
        raise ValueError(
            "modulus/density must be positive and rigidity/damping nonnegative"
        )
    if type(iterations) is not int or iterations < 2 or iterations % 2:
        raise ValueError("iterations must be a positive even integer")
    pins = set(pins)
    if any(type(i) is not int or not 0 <= i < n for i in pins):
        raise ValueError("pin index out of range")
    rest_normals = [[0.0, 0.0, 0.0] for _ in positions]
    masses = [0.0] * n
    edges = {}
    edge_area = {}
    incident = [[] for _ in positions]
    seen_faces = set()
    for f in range(len(triangles) // 3):
        a, b, c = triangles[3 * f : 3 * f + 3]
        key = tuple(sorted((a, b, c)))
        if key in seen_faces:
            raise ValueError("remove duplicate cloth triangles")
        seen_faces.add(key)
        area = (
            norm(
                cross(sub(positions[b], positions[a]), sub(positions[c], positions[a]))
            )
            / 2
        )
        if area < 1e-12:
            raise ValueError("remove degenerate cloth triangles")
        for v in (a, b, c):
            masses[v] += area * density / 3
            incident[v].append(f)
            face_normal = cross(
                sub(positions[b], positions[a]), sub(positions[c], positions[a])
            )
            rest_normals[v] = [x + y for x, y in zip(rest_normals[v], face_normal)]
        for x, y, z in [(a, b, c), (b, c, a), (c, a, b)]:
            key = tuple(sorted((x, y)))
            edges.setdefault(key, []).append((x, y, z))
            edge_area[key] = edge_area.get(key, 0) + area
    if any(m <= 0 for m in masses):
        raise ValueError("remove unused cloth vertices")
    stretch = []
    for e in edges:
        l = norm(sub(positions[e[1]], positions[e[0]]))
        width = 2 * edge_area[e] / (3 * l)
        stretch.append((e, (l, l / (stretch_modulus * width))))
    bends = []
    for edge, faces in edges.items():
        if len(faces) > 2:
            raise ValueError("cloth needs manifold edges; split nonmanifold vertices")
        if len(faces) == 2 and bending_rigidity > 0:
            (a, b, c), (x, y, d) = faces
            if (x, y) != (b, a):
                raise ValueError("orient cloth triangles consistently before building")
            e = sub(positions[b], positions[a])
            u = sub(positions[c], positions[a])
            v = sub(positions[d], positions[a])
            n1 = cross(e, u)
            n2 = cross(v, e)
            angle = math.atan2(dot(cross(n1, n2), e) / norm(e), dot(n1, n2))
            length = norm(e)
            width = 2 * edge_area[edge] / (3 * length)
            bends.append(((a, b, c, d), (angle, width / (bending_rigidity * length))))
    stretch, ec = colored(stretch)
    bends, bc = colored(bends)
    buffers = {}

    def buf(name, kind, data):
        buffers[name] = dict(type=kind, data=data or [0])

    buf("p", "vec3", positions)
    buf(
        "normal",
        "vec3",
        [
            [x / norm(v) for x in v] if norm(v) > 1e-12 else [0, 1, 0]
            for v in rest_normals
        ],
    )
    buf("tangent", "vec4", [[1, 0, 0, 1]] * n)
    for name in ["old", "velocity", "predicted"]:
        buf(name, "vec3", [[0, 0, 0]] * n)
    buf("indices", "u32", triangles)
    buf("triangles", "u32", triangles)
    buf("mass", "f32", [0 if i in pins else 1 / m for i, m in enumerate(masses)])
    for name, items, colors in [("edge", stretch, ec), ("bend", bends, bc)]:
        buf(
            "edges" if name == "edge" else "hinges",
            "u32",
            [i for ids, _ in items for i in ids],
        )
        buf("rest" if name == "edge" else "bend_rest", "f32", [r[0] for _, r in items])
        buf("lambda" if name == "edge" else "bend_lambda", "f32", [0] * len(items))
        buf(name + "_colors", "u32", colors)
        buf(
            "stretch_compliance" if name == "edge" else "bend_compliance",
            "f32",
            [r[1] for _, r in items],
        )
    offsets = [0]
    inc = []
    for group in incident:
        inc += group
        offsets.append(len(inc))
    buf("incident_offsets", "u32", offsets)
    buf("incidents", "u32", inc)
    neighbours = [[] for _ in positions]
    for a, b in edges:
        neighbours[a].append(b)
        neighbours[b].append(a)
    offsets = [0]
    items = []
    for row in neighbours:
        items += row
        offsets.append(len(items))
    buf("neighbour_offsets", "u32", offsets)
    buf("neighbours", "u32", items)
    faces_count = len(triangles) // 3
    self_leaves = 1 << (faces_count - 1).bit_length()
    buf("self_order", "u32", list(range(faces_count)))
    buf("snapshot", "vec3", positions)
    for key in ["self_lo", "self_hi"]:
        buf(key, "vec3", [[0, 0, 0]] * (2 * self_leaves))
    contact_capacity = 512
    for key in ["contact_key", "contact_ids", "contact_weights", "contact_plane"]:
        buf(key, "vec4", [[0, 0, 0, 0]] * (n * contact_capacity))
    for key in ["contact_kind", "contact_lambda", "contact_owner"]:
        buf(key, "f32", [0] * (n * contact_capacity))
    buf("contact_count", "f32", [0] * n)
    for key in ["contact_motion", "contact_tangent_lambda"]:
        buf(key, "vec3", [[0, 0, 0]] * (n * contact_capacity))
    if uvs is not None:
        if len(uvs) != n or any(
            len(v) != 2 or not all(math.isfinite(x) for x in v) for v in uvs
        ):
            raise ValueError("cloth TEXCOORD_0 needs one finite uv pair per vertex")
        buf("uv", "vec2", uvs)
    config = (
        "static const bool CLOTH_HAS_UV="
        + ("true" if uvs is not None else "false")
        + ";\n"
    )
    config += (
        "float2 cloth_uv(uint i){return uv_load(i);}"
        if uvs is not None
        else "float2 cloth_uv(uint i){return float2(0);}"
    ) + "\n"
    config += "".join(
        f"static const uint {k}={v}u;\n"
        for k, v in dict(
            CLOTH_VERTICES=n,
            CLOTH_EDGES=len(stretch),
            CLOTH_HINGES=len(bends),
            CLOTH_EDGE_COLORS=len(ec) - 1,
            CLOTH_BEND_COLORS=len(bc) - 1,
            CLOTH_ITERATIONS=iterations,
            CLOTH_ROUNDS=math.gcd(iterations, 8 if body else 2),
            CLOTH_FACES=faces_count,
            SELF_LEAVES=self_leaves,
            CONTACT_CAPACITY=contact_capacity,
        ).items()
    )
    config += f"static const float DAMPING={float(damping):.12g};\nstatic const float SELF_MARGIN={float(thickness * 3):.12g};\n"
    if body:
        name, bp, bt = body
        count = len(bt) // 3
        leaves = 1 << (count - 1).bit_length()
        if count < 1 or count > 4096:
            raise ValueError("body must have 1..4096 triangles")
        centers = [
            tuple(sum(bp[bt[3 * f + j]][k] for j in range(3)) / 3 for k in range(3))
            for f in range(count)
        ]

        def order(fs):
            if len(fs) < 2:
                return fs
            axis = max(
                range(3),
                key=lambda k: (
                    max(centers[f][k] for f in fs) - min(centers[f][k] for f in fs)
                ),
            )
            fs = sorted(fs, key=lambda f: centers[f][axis])
            # Full binary heap leaves need balanced power-of-two partitions.
            cut = 1 << ((len(fs) - 1).bit_length() - 1)
            return order(fs[:cut]) + order(fs[cut:])

        ordered = order(list(range(count)))
        inverse = [0] * count
        for leaf, face in enumerate(ordered):
            inverse[face] = leaf
        vertex_faces = [[] for _ in bp]
        for f in range(count):
            for v in bt[3 * f : 3 * f + 3]:
                vertex_faces[v].append(inverse[f])
        offsets = [0]
        inc = []
        for row in vertex_faces:
            inc += row
            offsets.append(len(inc))
        buf("collider_offsets", "u32", offsets)
        buf("collider_faces", "u32", inc)
        buf("collider_triangles", "vec3", [bt[3 * f : 3 * f + 3] for f in ordered])
        for key in ["collider_current", "collider_previous", "collider_normal"]:
            buf(key, "vec3", [[0, 0, 0]] * len(bp))
        config += f"static const uint COLLIDER_VERTICES={len(bp)}u;\n"
        for k in [
            "body_a",
            "body_b",
            "body_c",
            "body_old_a",
            "body_old_b",
            "body_old_c",
        ]:
            buf(k, "vec3", [[0, 0, 0]] * leaves)
        for k in ["body_lo", "body_hi"]:
            buf(k, "vec3", [[0, 0, 0]] * (2 * leaves))
        config += f"static const uint BODY_TRIANGLES={count}u;\nstatic const uint BODY_LEAVES={leaves}u;\nstatic const float THICKNESS={thickness};\n"
        contact = '#include "cloth/body.slang"\n'
    config += (
        ("struct Inputs {Geometry body;};\n" if body else "struct Inputs {};\n")
        + "float3 cloth_force(uint i,float3 p,float3 v,float time){"
        + force
        + "}\n"
    )
    solver = '#include "cloth/solver.slang"\n'
    contact = (
        contact
        or "void cloth_body_generate(uint i){}\nvoid cloth_collider_update(uint lane,Inputs inputs,bool initial){}\n"
    )
    metadata = [k for k, v in buffers.items() if v["type"] == "u32" and k != "indices"]
    for k in metadata:
        buffers[k]["type"] = "f32"
    config = (
        "".join(
            f"uint {k}_load(uint i){{return uint({k}_raw_load(i));}}\n"
            for k in metadata
        )
        + config
    )
    programs = {}
    materials = {}

    def program(name, entry, count):
        source = (
            config
            + solver
            + contact
            + '#include "cloth/self.slang"\n'
            + "\nvoid compute(ComputeContext ctx,Inputs inputs){"
            + entry
            + "}\n"
        )
        programs[name] = dict(
            source=source,
            schedule="simulation",
            dispatch=count,
            bindings={
                (k + "_raw" if k in metadata else k): k
                for k in buffers
                if k != "indices"
            },
        )
        materials[name] = dict(
            program=name,
            **(dict(inputs=dict(body=dict(geometry=body[0]))) if body else {}),
        )

    program("integrate", "cloth_integrate(ctx,inputs);", n)
    program("solve", "cloth_solve(ctx,inputs);", 64)
    program("publish", "cloth_publish(ctx,inputs);", n)
    contacts = []
    if body:
        program("refit", "cloth_collider_update(ctx.id.x,inputs,ctx.dt==0);", 64)
    program(
        "snapshot",
        "float3 p=p_load(ctx.id.x);if(all((asuint(p)&uint3(0x7f800000u))!=uint3(0x7f800000u)))snapshot_store(ctx.id.x,p);",
        n,
    )
    program("self_refit", "cloth_self_refit(ctx.id.x);", 64)
    program("self_generate", "cloth_self_generate(ctx.id.x);", n)
    contacts += ["snapshot", "self_refit", "self_generate"]
    init = ["integrate"] + (["refit"] if body else []) + ["publish"]
    steps = (
        ["integrate"]
        + (["refit"] if body else [])
        + (contacts + ["solve"]) * math.gcd(iterations, 8 if body else 2)
        + ["publish"]
    )
    return dict(
        buffers=buffers,
        programs=programs,
        materials=materials,
        geometry=dict(
            cloth=dict(
                attributes=dict(POSITION="p", NORMAL="normal", TANGENT="tangent"),
                indices="indices",
            )
        ),
        simulation=dict(
            t0=0,
            dt=dt,
            seed=17,
            state=[k for k in buffers if k != "indices"],
            init=init,
            steps=steps,
        ),
    )


def _node(scene, name):
    found = []

    def walk(nodes, parent):
        for node in nodes:
            if node.get("name") == name:
                found.append((node, parent))
            walk(node.get("children", []), node)

    walk(scene.get("nodes", []), None)
    if len(found) != 1:
        raise ValueError(
            f"cloth: name one unique native mesh node, found {len(found)} for {name!r}"
        )
    if found[0][0].get("type") != "mesh":
        raise ValueError(
            f"cloth {name!r}: use a native triangle mesh or a native collision proxy"
        )
    return found[0]


def _geometry(scene, node):
    shape = node.get("geometry")
    if isinstance(shape, str):
        shape = scene.get("geometry", {}).get(shape)
    if not isinstance(shape, dict) or "std" in shape:
        raise ValueError(
            f"cloth {node.get('name')!r}: expand geometry into native buffers before attaching cloth"
        )
    if (
        shape.get("topology", "triangles") != "triangles"
        or shape.get("face_vertex_counts")
        or shape.get("subdivision")
    ):
        raise ValueError(
            "cloth needs fixed triangles; triangulate and apply subdivision before attaching"
        )
    return shape


def _data(scene, column, kind, label):
    if isinstance(column, str):
        column = scene.get("buffers", {}).get(column)
    if (
        not isinstance(column, dict)
        or column.get("type") != kind
        or "data" not in column
    ):
        raise ValueError(
            f"cloth {label}: provide an in-memory {kind} data buffer before write_scene externalizes it"
        )
    return deepcopy(column["data"])


def attach(
    scene,
    node,
    *,
    pins=(),
    collider=None,
    dt=None,
    iterations=64,
    density=0.2,
    stretch_modulus=50000.0,
    bending_rigidity=0.002,
    damping=0.8,
    collision_margin=0.004,
    acceleration=None,
):
    """Attach compute cloth to an identity root mesh; preserve its surface material.

    Collider is a named native triangle mesh and may move or deform. Positions
    and pins are world metres. Density is kg/m², stretch modulus N/m, bending
    rigidity N·m; acceleration's Slang body returns m/s² including gravity.
    Omitted dt inherits scene.simulation.dt, otherwise defaults to 1/240 seconds.
    Existing simulation passes/state keep their order and share the same clock.
    """
    mesh, parent = _node(scene, node)
    if parent is not None or mesh.get("instances") is not None:
        raise ValueError(
            f"cloth {node!r}: use an identity root mesh for each cloth patch"
        )
    for field, identity in [
        ("translation", [0, 0, 0]),
        ("rotation", [0, 0, 0, 1]),
        ("scale", [1, 1, 1]),
    ]:
        if mesh.get(field, identity) != identity:
            raise ValueError(
                f"cloth {node!r}: bake {field} into the rest vertices and keep the cloth root at identity"
            )
    if mesh.get("target") is not None:
        raise ValueError("cloth root cannot aim at a target; use an identity root")
    for layer in scene.get("animation", []):
        for track in layer.get("tracks", []):
            path = track.get("path", [])
            if (
                path[:2] == ["nodes", node]
                and len(path) > 2
                and path[2] in ["translation", "rotation", "scale"]
            ):
                raise ValueError(
                    "animate cloth pins through skin/morphs, not the cloth root transform"
                )
    shape = _geometry(scene, mesh)
    positions = _data(
        scene, shape.get("attributes", {}).get("POSITION"), "vec3", node + " POSITION"
    )
    if positions and isinstance(positions[0], (int, float)):
        if len(positions) % 3:
            raise ValueError("POSITION needs xyz triples")
        positions = [positions[i : i + 3] for i in range(0, len(positions), 3)]
    indices = (
        _data(scene, shape["indices"], "u32", node + " indices")
        if "indices" in shape
        else list(range(len(positions)))
    )
    body = None
    if collider is not None:
        if collider == node:
            raise ValueError(
                "choose an independent collider; a cloth cannot reference itself"
            )
        body_node, _ = _node(scene, collider)
        body_shape = _geometry(scene, body_node)
        bp = _data(
            scene,
            body_shape.get("attributes", {}).get("POSITION"),
            "vec3",
            collider + " POSITION",
        )
        if bp and isinstance(bp[0], (int, float)):
            if len(bp) % 3:
                raise ValueError("collider POSITION needs xyz triples")
            bp = [bp[i : i + 3] for i in range(0, len(bp), 3)]
        bt = (
            _data(scene, body_shape["indices"], "u32", collider + " indices")
            if "indices" in body_shape
            else list(range(len(bp)))
        )
        body = (collider, bp, bt)
    if not body is None:
        if (
            not bp
            or len(bt) % 3
            or any(type(i) is not int or not 0 <= i < len(bp) for i in bt)
        ):
            raise ValueError("cloth collider: triangulate valid native geometry")
        if any(len(p) != 3 or not all(math.isfinite(x) for x in p) for p in bp):
            raise ValueError("cloth collider POSITION needs finite xyz triples")
    existing = scene.get("simulation")
    step = dt if dt is not None else (existing.get("dt") if existing else "1/240")
    try:
        period = Fraction(str(step))
    except (ValueError, ZeroDivisionError):
        raise ValueError(
            "cloth dt needs positive seconds or a fraction such as 1/240"
        ) from None
    if period <= 0:
        raise ValueError("cloth dt must be positive")
    if existing and Fraction(str(existing["dt"])) != period:
        raise ValueError(
            "cloth must share scene.simulation.dt; omit dt or use that exact clock"
        )
    prefix = "cloth_" + hashlib.sha256(node.encode()).hexdigest()[:12] + "_"
    for table in ["buffers", "programs", "materials", "geometry"]:
        if any(k.startswith(prefix) for k in scene.get(table, {})):
            raise ValueError(
                f"cloth {node!r} is already attached; edit or rebuild the source scene"
            )
    options = dict(
        positions=positions,
        triangles=indices,
        pins=pins,
        body=body,
        dt=step,
        iterations=iterations,
        density=density,
        stretch_modulus=stretch_modulus,
        bending_rigidity=bending_rigidity,
        damping=damping,
        thickness=collision_margin,
    )
    if acceleration is not None:
        if not isinstance(acceleration, str):
            raise ValueError(
                "acceleration is the Slang body of the acceleration callback"
            )
        options["force"] = acceleration
    uv = shape.get("attributes", {}).get("TEXCOORD_0")
    if uv is not None:
        uvs = _data(scene, uv, "vec2", node + " TEXCOORD_0")
        if uvs and isinstance(uvs[0], (int, float)):
            uvs = [uvs[i : i + 2] for i in range(0, len(uvs), 2)]
        options["uvs"] = uvs
    fragment = _build(**options)
    normal = shape.get("attributes", {}).get("NORMAL")
    if normal is not None:
        normals = _data(scene, normal, "vec3", node + " NORMAL")
        if normals and isinstance(normals[0], (int, float)):
            normals = [normals[i : i + 3] for i in range(0, len(normals), 3)]
        if len(normals) != len(positions) or any(
            len(v) != 3 or not all(math.isfinite(x) for x in v) for v in normals
        ):
            raise ValueError("cloth NORMAL needs one finite xyz triple per vertex")
        fragment["buffers"]["normal"]["data"] = normals
    remap = lambda key: prefix + key
    programs = {
        remap(k): dict(
            v,
            bindings={alias: remap(target) for alias, target in v["bindings"].items()},
        )
        for k, v in fragment["programs"].items()
    }
    materials = {
        remap(k): dict(v, program=remap(v["program"]))
        for k, v in fragment["materials"].items()
    }
    geometry = deepcopy(shape)
    geometry["attributes"].update(
        {k: remap(v) for k, v in fragment["geometry"]["cloth"]["attributes"].items()}
    )
    geometry["indices"] = remap("indices")
    # All validation/build work precedes mutation of the author's scene.
    for table, values in [
        ("buffers", {remap(k): v for k, v in fragment["buffers"].items()}),
        ("programs", programs),
        ("materials", materials),
        ("geometry", {remap("mesh"): geometry}),
    ]:
        scene.setdefault(table, {}).update(values)
    mesh["geometry"] = remap("mesh")
    if not existing:
        scene["simulation"] = {
            k: v
            for k, v in fragment["simulation"].items()
            if k not in ["state", "init", "steps"]
        }
        existing = scene["simulation"]
    for field in ["state", "init", "steps"]:
        existing.setdefault(field, []).extend(
            remap(k) for k in fragment["simulation"][field]
        )
