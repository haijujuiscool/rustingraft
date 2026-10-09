"""Print size/bounds/animations of glTF/GLB files from accessor min/max.

Usage: python -I tools/assets/bounds.py <file-or-dir>... [--json]
Bounds are world-space (node TRS applied to accessor bbox corners), skinning ignored.
"""
import json
import math
import os
import struct
import sys


def load_gltf(path):
    with open(path, "rb") as f:
        data = f.read()
    if data[:4] == b"glTF":
        length = struct.unpack_from("<I", data, 12)[0]
        return json.loads(data[20:20 + length])
    return json.loads(data)


def mat_mul(a, b):
    return [[sum(a[i][k] * b[k][j] for k in range(4)) for j in range(4)] for i in range(4)]


def node_matrix(node):
    if "matrix" in node:
        m = node["matrix"]  # column-major
        return [[m[c * 4 + r] for c in range(4)] for r in range(4)]
    tx, ty, tz = node.get("translation", [0, 0, 0])
    x, y, z, w = node.get("rotation", [0, 0, 0, 1])
    sx, sy, sz = node.get("scale", [1, 1, 1])
    r = [
        [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
        [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
        [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)],
    ]
    return [
        [r[0][0] * sx, r[0][1] * sy, r[0][2] * sz, tx],
        [r[1][0] * sx, r[1][1] * sy, r[1][2] * sz, ty],
        [r[2][0] * sx, r[2][1] * sy, r[2][2] * sz, tz],
        [0, 0, 0, 1],
    ]


IDENTITY = [[1 if i == j else 0 for j in range(4)] for i in range(4)]


def bounds(gltf):
    lo = [math.inf] * 3
    hi = [-math.inf] * 3
    nodes = gltf.get("nodes", [])

    def visit(idx, parent):
        node = nodes[idx]
        m = mat_mul(parent, node_matrix(node))
        if "mesh" in node:
            for prim in gltf["meshes"][node["mesh"]]["primitives"]:
                acc = gltf["accessors"][prim["attributes"]["POSITION"]]
                amin, amax = acc.get("min"), acc.get("max")
                if not amin:
                    continue
                for cx in (amin[0], amax[0]):
                    for cy in (amin[1], amax[1]):
                        for cz in (amin[2], amax[2]):
                            p = [m[i][0] * cx + m[i][1] * cy + m[i][2] * cz + m[i][3] for i in range(3)]
                            for i in range(3):
                                lo[i] = min(lo[i], p[i])
                                hi[i] = max(hi[i], p[i])
        for c in node.get("children", []):
            visit(c, m)

    scene = gltf.get("scenes", [{}])[gltf.get("scene", 0)] if gltf.get("scenes") else {}
    roots = scene.get("nodes", range(len(nodes)))
    for r in roots:
        visit(r, IDENTITY)
    return lo, hi


def info(path):
    g = load_gltf(path)
    lo, hi = bounds(g)
    size = [round(hi[i] - lo[i], 3) for i in range(3)] if lo[0] != math.inf else None
    return {
        "file": path.replace("\\", "/"),
        "size_xyz": size,
        "min": [round(v, 3) for v in lo] if size else None,
        "max": [round(v, 3) for v in hi] if size else None,
        "skinned": bool(g.get("skins")),
        "animations": [a.get("name", f"#{i}") for i, a in enumerate(g.get("animations", []))],
    }


def main(argv):
    as_json = "--json" in argv
    paths = []
    for a in (x for x in argv if x != "--json"):
        if os.path.isdir(a):
            for root, _, files in os.walk(a):
                paths += [os.path.join(root, f) for f in sorted(files) if f.endswith((".glb", ".gltf"))]
        else:
            paths.append(a)
    rows = [info(p) for p in paths]
    if as_json:
        print(json.dumps(rows, indent=1))
        return
    for r in rows:
        anims = f" anims={','.join(r['animations'])}" if r["animations"] else ""
        print(f"{r['file']}: size={r['size_xyz']} min={r['min']} max={r['max']}"
              f"{' skinned' if r['skinned'] else ''}{anims}")


def _selftest():
    g = {"nodes": [{"mesh": 0, "translation": [0, 1, 0], "scale": [2, 2, 2]}],
         "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
         "accessors": [{"min": [-0.5, 0, -0.5], "max": [0.5, 1, 0.5]}]}
    lo, hi = bounds(g)
    assert lo == [-1, 1, -1] and hi == [1, 3, 1], (lo, hi)


if __name__ == "__main__":
    _selftest()
    main(sys.argv[1:])
