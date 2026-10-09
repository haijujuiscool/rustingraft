"""Rebuild scenes/main.rscene (and docs/scene-names.md) for Rusting Raft from scratch.

    python -I tools/scene/build_scene.py             # scene + names doc
    python -I tools/scene/build_scene.py --capture   # also render check views to build/capture/*.png
    python -I tools/scene/build_scene.py --capture origin hook   # only the named views
    python -I tools/scene/build_scene.py --capture-only hook     # captures from the last built scene

Every run writes a fresh base scene and applies one patch (build/scene/main.patch.json)
with `rusting scene patch`, so the output only depends on this file, sim/src/content.rs
(islands, spots, enum variants) and the GLBs in assets/models. Entity ids are uuid5 of the
entity name, so ids stay stable between runs. Each GLB is imported once into
build/scene/protos/ with `rusting scene add-model` (redone when the GLB is newer) and its
entities are cloned from there under our own names.

Set RUSTING to the CLI path if the engine is not at the default location.
"""
import copy
import json
import math
import os
import random
import re
import subprocess
import sys
import uuid
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "assets"))
import bounds as gltf_bounds  # noqa: E402  (tools/assets/bounds.py)

RUSTING = os.environ.get("RUSTING", r"C:\Users\juliu\RustingEngine\target\release\rusting.exe")
SCENE = ROOT / "scenes" / "main.rscene"
WORK = ROOT / "build" / "scene"
PROTOS = WORK / "protos"
CAPTURE = ROOT / "build" / "capture"
DOCS = ROOT / "docs" / "scene-names.md"
CONTENT = ROOT / "sim" / "src" / "content.rs"
MODELS_DIR = ROOT / "assets" / "models"

NS = uuid.uuid5(uuid.NAMESPACE_URL, "rustingraft/scenes/main")
TAU = 2 * math.pi
PI = math.pi
HIDE_Y = -500.0
DECK_Y = 0.45  # content.rs DECK_Y
FIT_HUMAN = 1.8

# palette (linear-ish base colours)
SAND = (0.86, 0.75, 0.52)
SAND_DARK = (0.6, 0.53, 0.42)
GRASS = (0.38, 0.6, 0.26)
GRASS_DARK = (0.3, 0.48, 0.22)
ROCK = (0.5, 0.48, 0.45)
WOOD = (0.55, 0.37, 0.21)
WOOD_DARK = (0.4, 0.27, 0.16)
STEEL = (0.42, 0.44, 0.47)
STEEL_DARK = (0.28, 0.29, 0.31)
RUST = (0.56, 0.3, 0.16)
RUST_TINT = (1.0, 0.72, 0.55)
CORAL = (0.95, 0.5, 0.45)
WHITE = (0.92, 0.92, 0.9)

DOC = {}  # section -> [(name, text)]


def note(section, name, text):
    DOC.setdefault(section, []).append((name, text))


# ---------------------------------------------------------------- small math

def r4(v):
    return round(float(v), 4) + 0.0


def vec(v):
    return [r4(x) for x in v]


def as3(s):
    return (s, s, s) if isinstance(s, (int, float)) else tuple(s)


def rot_apply(rot, v):
    """Engine euler order: R = Rz * Ry * Rx (Rx applied first)."""
    rx, ry, rz = rot
    x, y, z = v
    c, s = math.cos(rx), math.sin(rx)
    y, z = c * y - s * z, s * y + c * z
    c, s = math.cos(ry), math.sin(ry)
    x, z = c * x + s * z, -s * x + c * z
    c, s = math.cos(rz), math.sin(rz)
    x, y = c * x - s * y, s * x + c * y
    return (x, y, z)


def corners(lo, hi):
    return [(a, b, c) for a in (lo[0], hi[0]) for b in (lo[1], hi[1]) for c in (lo[2], hi[2])]


def xform(t, p):
    s = t["scale"]
    q = rot_apply(t["rotation"], (p[0] * s[0], p[1] * s[1], p[2] * s[2]))
    return tuple(q[i] + t["position"][i] for i in range(3))


def bbox(points):
    return ([min(p[i] for p in points) for i in range(3)], [max(p[i] for p in points) for i in range(3)])


def yaw_to(dx, dz):
    """Yaw that turns an object's -Z (its forward) toward (dx, dz)."""
    return math.atan2(-dx, -dz)


# ---------------------------------------------------------------- entities

ENTS = []
E = {}
BOX = {}  # entity name -> local (lo, hi) of its visible geometry, for bounds in docs


def ent(name, parent=None, pos=(0, 0, 0), rot=(0, 0, 0), scale=1.0, visible=True, classes=(), **sec):
    assert name not in E, f"duplicate name {name}"
    assert parent is None or parent in E, f"{name}: unknown parent {parent}"
    e = {"id": str(uuid.uuid5(NS, name)), "name": name}
    if parent:
        e["parent"] = parent
    e["transform"] = {"position": vec(pos), "rotation": vec(rot), "scale": vec(as3(scale))}
    if not visible:
        e["visible"] = False
    if classes:
        e["classes"] = list(classes)
    for k, v in sec.items():
        if v is not None:
            e[k] = v
    ENTS.append(e)
    E[name] = e
    return name


def mat(color, rough=0.85, metal=0.0, emissive=(0, 0, 0), unlit=False):
    c = list(color) + ([1.0] if len(color) == 3 else [])
    return {"Inline": {"name": "", "model": "Unlit" if unlit else "Pbr",
                       "alpha_mode": "Blend" if c[3] < 1 else "Opaque", "base_color": vec(c),
                       "emissive": vec(emissive), "metallic": metal, "roughness": rough}}


def fixed(shape, friction=0.8):
    return dict(physics_body={"simulation": "Cpu", "solver": "Simplified"}, rigid_body={"kind": "Fixed"},
                collider={"shape": shape, "friction": friction, "restitution": 0.0, "sensor": False})


def box_shape(hx, hy, hz):
    return {"Box": {"half_extents": vec((hx, hy, hz))}}


PRIM_BOX = {"Capsule": ((-.5, -1, -.5), (.5, 1, .5))}


def prim(name, parent, shape, pos, scale, color, rot=(0, 0, 0), rough=0.85, metal=0.0, emissive=(0, 0, 0),
         collide=None, ground=False, shadows=True, visible=True, unlit=False):
    sec = {"mesh_renderer": {"mesh": {"BuiltinPrimitive": shape},
                             "material": mat(color, rough, metal, emissive, unlit),
                             "cast_shadows": shadows, "receive_shadows": True}}
    if collide == "box":
        sec.update(fixed(box_shape(.5, .5, .5)))
    elif collide == "convex":
        sec.update(fixed("ConvexMesh"))
    ent(name, parent, pos, rot, scale, visible, ("ground",) if ground else (), **sec)
    BOX[name] = PRIM_BOX.get(shape, ((-.5, -.5, -.5), (.5, .5, .5)))
    return name


def collider(name, parent, center, half, ground=False):
    """Invisible Fixed box (colliders sit on the entity origin, so offsets need a child)."""
    return ent(name, parent, center, classes=("ground",) if ground else (), **fixed(box_shape(*half)))


def beam(name, parent, p0, p1, t, color, shape="Cylinder", **kw):
    v = [p1[i] - p0[i] for i in range(3)]
    length = math.sqrt(sum(x * x for x in v))
    u = [x / length for x in v]
    a = math.asin(max(-1.0, min(1.0, u[2])))
    b = math.atan2(-u[0], u[1])
    mid = [(p0[i] + p1[i]) / 2 for i in range(3)]
    return prim(name, parent, shape, mid, (t, length, t), color, rot=(a, 0, b), **kw)


def ramp(name, parent, bottom, top, width, thick, color, **kw):
    """Walkable slab whose top face runs from `bottom` to `top` (Box collider, class ground)."""
    dx, dy, dz = (top[i] - bottom[i] for i in range(3))
    run = math.hypot(dx, dz)
    phi = math.atan2(dy, run)
    yaw = math.atan2(dx, dz)
    n = (-math.sin(phi) * math.sin(yaw), math.cos(phi), -math.sin(phi) * math.cos(yaw))
    mid = [(bottom[i] + top[i]) / 2 - n[i] * thick / 2 for i in range(3)]
    length = math.hypot(run, dy)
    return prim(name, parent, "Cube", mid, (width, thick, length), color, rot=(-phi, yaw, 0),
                collide="box", ground=True, **kw)


def emitter(name, parent, pos, comp, visible=True):
    return ent(name, parent, pos, visible=visible, components={"rusting.particle_emitter": comp})


def light(name, parent, pos, color, intensity, rng, visible=True):
    return ent(name, parent, pos, visible=visible,
               point_light={"color": vec(color), "intensity": intensity, "range": rng})


def subtree(root):
    kids = {}
    for e in ENTS:
        kids.setdefault(e.get("parent"), []).append(e["name"])
    out, stack = [], [root]
    while stack:
        n = stack.pop()
        out.append(n)
        stack.extend(reversed(kids.get(n, [])))
    return out


def bounds_of(root):
    """Bounds of everything under `root`, in root's local space (root transform excluded)."""
    pts = []
    for n in subtree(root):
        if n not in BOX or n == root and not E[n].get("parent") and False:
            continue
        if n not in BOX:
            continue
        cs = corners(*BOX[n])
        m = n
        while m != root:
            cs = [xform(E[m]["transform"], p) for p in cs]
            m = E[m]["parent"]
        if n == root:  # geometry on the root itself sits in its own space
            pass
        pts += cs
    return bbox(pts) if pts else ([0, 0, 0], [0, 0, 0])


# ---------------------------------------------------------------- models

def run(cmd, check=True):
    p = subprocess.run([str(c) for c in cmd], capture_output=True, text=True, encoding="utf-8", errors="replace")
    if check and p.returncode != 0:
        sys.exit(f"command failed ({p.returncode}): {' '.join(map(str, cmd))}\n{p.stdout[-3000:]}\n{p.stderr[-3000:]}")
    return p


def base_scene(name):
    return {"format_version": 9, "name": name, "entities": [],
            "render": {"quality": "Balanced", "culling": "Auto"}, "simulation": {"determinism": "Off"}}


ASSET_RE = re.compile(r"^(?:\.\.[\\/])+assets[\\/]")


def fix_paths(obj, prefix):
    if isinstance(obj, dict):
        return {k: fix_paths(v, prefix) for k, v in obj.items()}
    if isinstance(obj, list):
        return [fix_paths(v, prefix) for v in obj]
    if isinstance(obj, str) and ASSET_RE.match(obj):
        return ASSET_RE.sub(lambda _m: prefix, obj)
    return obj


PROTO_CACHE = {}


def proto(rel):
    if rel in PROTO_CACHE:
        return PROTO_CACHE[rel]
    glb = MODELS_DIR / rel
    assert glb.exists(), glb
    slug = re.sub(r"[^a-z0-9]+", "_", rel.lower().rsplit(".", 1)[0]).strip("_")
    path = PROTOS / f"{slug}.rscene"
    if not path.exists() or path.stat().st_mtime < glb.stat().st_mtime:
        path.write_text(json.dumps(base_scene(slug)), encoding="utf-8")
        run([RUSTING, "scene", "add-model", path, glb, "--name", "P", "--json"])
    ents = json.loads(path.read_text(encoding="utf-8"))["entities"]
    for e in ents:
        e["components"] = {k: json.loads(v) if isinstance(v, str) else v
                           for k, v in (e.get("components") or {}).items()}
    byid = {e["id"]: e for e in ents}
    order, seen = [], set()

    def visit(e):
        if e["id"] in seen:
            return
        if e.get("parent"):
            visit(byid[e["parent"]])
        seen.add(e["id"])
        order.append(e)

    for e in ents:
        visit(e)
    info = gltf_bounds.info(str(glb))
    PROTO_CACHE[rel] = (order, info)
    return PROTO_CACHE[rel]


ONCE = re.compile(r"^(die|death|pick-up|attack|interact|emote|punch|bite|hit-|recieve|receive)|-shoot$")


def clip_key(raw):
    parts = [p for p in raw.split("|") if p.strip() and "armature" not in p.lower()]
    return re.sub(r"[\s_]+", "-", (parts[0] if parts else raw).strip()).lower()


def is_identity(t):
    return (all(abs(x) < 1e-6 for x in t["position"]) and all(abs(x) < 1e-6 for x in t["rotation"])
            and all(abs(x - 1) < 1e-6 for x in t["scale"]))


def model(rel, name, parent=None, pos=(0, 0, 0), rot=(0, 0, 0), scale=1.0, anchor="base", tint=None,
          emissive=None, visible=True, shadows=True, solid=False, ground=False, anim="keep", autoplay=None,
          host=None, classes=()):
    """Clone a GLB under `name`. anchor: base (bbox bottom-centre at pos), y (bottom only),
    center, top (top-centre) or origin (model origin at pos). `host` moves the animation to
    that ancestor (an outer group code rotates/plays on). Returns the clip names."""
    order, info = proto(rel)
    order = [dict(e) for e in order]
    scale = as3(scale)
    lo, hi = info["min"], info["max"]
    if anchor != "origin":
        mn, mx = bbox([rot_apply(rot, (p[0] * scale[0], p[1] * scale[1], p[2] * scale[2])) for p in corners(lo, hi)])
        c = [(mn[i] + mx[i]) / 2 for i in range(3)]
        off = {"base": (-c[0], -mn[1], -c[2]), "y": (0, -mn[1], 0), "center": (-c[0], -c[1], -c[2]),
               "top": (-c[0], -mx[1], -c[2])}[anchor]
        pos = tuple(pos[i] + off[i] for i in range(3))
    has_anim = any(e["components"].get("rusting.animation") for e in order) and anim != "drop"
    has_skin = any(e["components"].get("rusting.skin") for e in order)
    # collapse an empty root with one identity child (static props only)
    while not has_anim and not has_skin:
        root = order[0]
        kids = [e for e in order if e.get("parent") == root["id"]]
        if (root.get("mesh_renderer") or [k for k in root["components"] if k != "rusting.animation"]
                or len(kids) != 1 or not is_identity(kids[0]["transform"]) or not is_identity(root["transform"])):
            break
        kid = dict(kids[0])
        kid["parent"] = None
        order = [kid] + [e for e in order if e is not root and e is not kids[0]]
        order = [dict(e, parent=root["parent"] if e.get("parent") == kid["id"] and False else e.get("parent"))
                 for e in order]
    root = order[0]
    names, used = {}, set()
    for e in order:
        if e is root:
            n = name
        else:
            n = f"{name}.{e['name']}"
            k = 2
            while n in used or n in E:
                n, k = f"{name}.{e['name']}.{k}", k + 1
        used.add(n)
        names[e["id"]] = n
    local = {}
    for e in order:
        local.setdefault(e["name"], names[e["id"]])

    def remap(path):
        return "/".join(s if s in ("", ".", "..") else local.get(s, s) for s in path.split("/"))

    clips = []
    moved = None
    for e in order:
        n = names[e["id"]]
        comps = {}
        for k, v in e["components"].items():
            v = copy.deepcopy(v)
            if k == "rusting.animation":
                if anim == "drop":
                    continue
                keyed = {}
                out = []
                for c in v.get("clips", []):
                    key = clip_key(c["name"])
                    if key.startswith("wheelchair"):
                        continue
                    base, i = key, 2
                    while key in keyed:
                        key, i = f"{base}-{i}", i + 1
                    keyed[c["name"]] = key
                    c["name"] = key
                    if ONCE.search(key):
                        c["repeat"] = "Once"
                    for tr in c.get("tracks", []):
                        tr["target"] = remap(tr["target"])
                    out.append(c)
                v["clips"] = out
                if v.get("root_bone"):
                    v["root_bone"] = remap(v["root_bone"])
                names_out = [c["name"] for c in out]
                if autoplay is None:
                    ap = "idle" if "idle" in names_out else keyed.get(v.get("autoplay", ""), "")
                else:
                    ap = autoplay
                assert ap == "" or ap in names_out, f"{name}: no clip {ap!r} in {names_out}"
                v["autoplay"] = ap
                clips += names_out
                if host:
                    assert e is root, "host only supports an animation on the model root"
                    pre = f"{name}/"
                    for c in out:
                        for tr in c.get("tracks", []):
                            tr["target"] = pre + tr["target"] if tr["target"] else name
                    if v.get("root_bone"):
                        v["root_bone"] = pre + v["root_bone"]
                    moved = v
                    continue
            elif k == "rusting.skin":
                v["joints"] = [remap(j) for j in v["joints"]]
            comps[k] = v
        sec = {}
        mr = copy.deepcopy(e.get("mesh_renderer"))
        if mr:
            m = mr["material"].get("Inline")
            if m and tint:
                m["base_color"] = vec([m["base_color"][i] * tint[i] for i in range(3)] + [m["base_color"][3]])
            if m and emissive:
                m["emissive"] = vec(emissive)
            mr["cast_shadows"] = bool(shadows and mr.get("cast_shadows", True))
            sec["mesh_renderer"] = fix_paths(mr, "..\\assets\\")
            if solid or ground:
                sec.update(fixed("ConvexMesh"))
        if comps:
            sec["components"] = comps
        cls = (("ground",) if ground and mr else ()) + (tuple(classes) if e is root else ())
        if e is root:
            ent(n, parent, pos, rot, scale, visible, cls, **sec)
        else:
            t = e["transform"]
            ent(n, names[e["parent"]], t["position"], t["rotation"], t["scale"], True, cls, **sec)
    if moved is not None:
        E[host].setdefault("components", {})["rusting.animation"] = moved
    BOX[name] = (lo, hi)
    return clips


def model_height(rel):
    info = proto(rel)[1]
    return info["max"][1] - info["min"][1]


def human(rel, name, parent, pos, yaw=0.0, height=FIT_HUMAN, visible=True, autoplay="idle", tint=None):
    """Group `name` (forward -Z, feet at its origin) holding the model turned to face -Z; the
    animation sits on the group so code plays clips on `name`."""
    ent(name, parent, pos, (0, yaw, 0), visible=visible)
    s = height / model_height(rel)
    return model(rel, f"{name}.model", name, (0, 0, 0), (0, PI, 0), s, host=name, autoplay=autoplay, tint=tint)


# ---------------------------------------------------------------- content.rs

def snake(s):
    s = re.sub(r"[^A-Za-z0-9]+", "_", s.strip())
    return re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "_", s).lower().strip("_")


SRC = CONTENT.read_text(encoding="utf-8")


def enum_variants(name):
    body = re.search(rf"pub enum {name} \{{(.*?)\n\}}", SRC, re.S).group(1)
    return re.findall(r"^\s*([A-Z]\w*)\s*[,(]", body, re.M)


def islands():
    body = SRC.split("pub const ISLANDS", 1)[1].split("\n];", 1)[0]
    out = []
    for blk in body.split("IslandDef {")[1:]:
        def f(k):
            return re.search(rf"\b{k}:\s*([^,\n]+)", blk).group(1).strip()
        out.append(dict(name=f("name").strip('"'), x=float(f("x")), z=float(f("z")), r=float(f("radius")),
                        tier=int(f("tier")), style=f("style").split("::")[-1], survivor=f("survivor") == "true",
                        shrines=int(f("shrines")), chest=not re.search(r"\bchest:\s*&\[\s*\]", blk)))
    assert len(out) == 15, len(out)
    return out


def spot_factors():
    def pair(spot):
        m = re.search(rf"Spot::{spot} => \(([-\d.]+)(\s*\*\s*r)?,\s*([-\d.]+)(\s*\*\s*r)?\)", SRC)
        return (float(m.group(1)), bool(m.group(2)), float(m.group(3)), bool(m.group(4)))
    sp = {k: pair(k) for k in ("Chest", "Survivor", "Generator", "Beacon", "Barnaby")}
    assert "0.7 * r * a.cos(), 0.7 * r * a.sin()" in SRC and "i as f32 * TAU / 3.0" in SRC, "Shrine spot changed"
    return sp


SPOTS = spot_factors()


def spot(d, kind, i=0):
    if kind == "Shrine":
        a = i * TAU / 3
        return (0.7 * d["r"] * math.cos(a), 0.7 * d["r"] * math.sin(a))
    fx, rx, fz, rz = SPOTS[kind]
    return (fx * (d["r"] if rx else 1), fz * (d["r"] if rz else 1))


# ---------------------------------------------------------------- look, player, input

def effects():
    p = run([RUSTING, "effect", "list", "--json"])
    return {e["name"]: e["component"] for e in json.loads(p.stdout)["data"]["effects"]}


FX = effects()


def fx(name, **over):
    c = copy.deepcopy(FX[name])
    c.update(over)
    return c


def build_look():
    ent("Sun", pos=(0, 60, 0), rot=(-0.85, 0.6, 0),
        directional_light={"color": [1.0, 0.94, 0.84], "illuminance": 100000.0, "shadows": True})
    ent("Sky Light", components={"rusting.sky_light": {"sky_color": [0.55, 0.7, 1.0],
                                                      "ground_color": [0.42, 0.38, 0.3], "intensity": 0.4}})
    ent("Ambient Light", components={"rusting.ambient_light": {"color": [0.85, 0.9, 1.0], "intensity": 0.08}})
    ent("Fog", components={"rusting.fog": {"color": [0.6, 0.74, 0.9], "density": 0.0026, "height": 0.0,
                                           "height_falloff": 0.012, "sun_scatter": 0.35, "sky_affect": 0.55}})
    ent("Background", components={"rusting.background": {"color": [0.36, 0.58, 0.86, 1.0]}})
    ent("Post", components={
        "rusting.tone_mapping": {"mapper": "Aces", "exposure": 1.0},
        "rusting.bloom": {"intensity": 0.12, "threshold": 1.4, "spread": 0.5},
        "rusting.color_grading": {"contrast": 1.06, "saturation": 1.1, "shadows": [0.96, 0.99, 1.05],
                                  "highlights": [1.04, 1.01, 0.96], "vignette": 0.12},
        "rusting.ambient_occlusion": {"radius": 0.8, "intensity": 1.2},
    })
    for n, t in [
        ("Sun", "directional_light, warm key, shadows on (the one shadow caster). Night/dusk: set_light or "
                "set_field `/directional_light/illuminance` and `/transform/rotation`."),
        ("Sky Light", "rusting.sky_light, cool sky / warm ground fill."),
        ("Ambient Light", "rusting.ambient_light, flat fill."),
        ("Fog", "rusting.fog: density 0.0026 keeps Hook Isle (38 m) clear, islands 300-700 m away faint "
                "and >1.2 km hidden. Night: lower `/components/rusting.fog/color`."),
        ("Background", "rusting.background clear colour (sky)."),
        ("Post", "rusting.tone_mapping (Aces), rusting.bloom (mild), rusting.color_grading (slightly warm), "
                 "rusting.ambient_occlusion (off on Eco)."),
    ]:
        note("Look (lights, fog, post)", n, t)


INPUTS = [("interact", ["KeyE"]), ("use", ["MouseLeft"]), ("aim", ["MouseRight"]), ("camera", ["KeyV"]),
          ("sac", ["Tab"]), ("build", ["KeyB"]), ("rotate", ["KeyR"]), ("pause", ["Escape"])] + \
         [(f"slot{i}", [f"Digit{i}"]) for i in range(1, 7)]


def build_player():
    pc = {"walk_speed": 4.5, "sprint_multiplier": r4(7.0 / 4.5), "jump_speed": 5.0, "air_jumps": 0,
          "gravity": 9.81, "look_sensitivity": 0.002, "mouse_look": True, "collision_mask": 4294967295,
          "yaw": 0.0, "pitch": -0.25, "pitch_limits": [-1.4, 1.2], "yaw_limits": None,
          "camera_distance": 4.5, "camera_height": 0.6, "camera_offset": [0.0, 0.0, 0.0],
          "max_slope": 0.7853982, "max_step_height": 0.35, "push_bodies": True, "turn_speed": 10.0,
          "crouch_height": 0.0, "crouch_multiplier": 0.5}
    ent("Player", pos=(0, DECK_Y + 0.9 + 0.01, 0),
        physics_body={"simulation": "Cpu", "solver": "Simplified"},
        rigid_body={"kind": "Kinematic"},
        collider={"shape": {"Capsule": {"half_height": 0.6, "radius": 0.3}}, "friction": 0.5,
                  "restitution": 0.0, "sensor": False},
        components={"rusting.player_controller": pc})
    ent("Game Camera", "Player", (0, 0.7, 0),
        camera={"projection": {"Perspective": {"vertical_fov_radians": 1.0471976, "near": 0.05, "far": 1500.0}},
                "active": True, "priority": 10})
    clips = human("character/kenney-mini/character-male-a.glb", "Player Avatar", "Player", (0, -0.9, 0))
    collider("Spawn Platform", None, (0, DECK_Y - 0.1, 0), (4.0, 0.1, 4.0))
    note("Player and camera", "Player",
         "rusting.player_controller on a Kinematic capsule (half_height 0.6, radius 0.3, feet 0.9 below the "
         f"origin). Spawn ({0}, {r4(DECK_Y + 0.91)}, 0) on the deck. walk_speed 4.5, sprint_multiplier "
         f"{r4(7 / 4.5)} (= 7 m/s, held `player.sprint`), jump_speed 5 (~1.3 m), max_step_height 0.35, "
         "turn_speed 10 (turns `Player Avatar` to the walk direction). yaw 0 looks toward -Z (Hook Isle).")
    note("Player and camera", "Game Camera",
         "Active camera, child of Player at (0, 0.7, 0), fov 60 deg, far 1500. Third person by default "
         "(camera_distance 4.5, camera_height 0.6). First/third-person toggle (action `camera`, V) is code: "
         "first person = `/components/rusting.player_controller/camera_distance` 0 via set_field or "
         "set_player, plus set_visible(\"Player Avatar\", false); third person = 4.5 and visible again.")
    note("Player and camera", "Player Avatar",
         "Group at (0, -0.9, 0) under Player (feet), forward -Z; holds `Player Avatar.model` (Kenney mini "
         f"male A, 1.8 m). Animation is on `Player Avatar`; clips: {', '.join(clips)}.")
    note("Player and camera", "Spawn Platform",
         "Invisible Fixed box, top at y 0.45, x/z in [-4, 4] (the 4x4 start raft). Holds the player up "
         "before the first raft tiles exist. Despawn it once raft tiles are spawned (the raft drifts away "
         "from it).")
    for a, keys in INPUTS:
        ent(f"input.{a}", components={"rusting.input_action": {"action": a, "inputs": keys}})
        note("Input actions", f"input.{a}", f"action `{a}`: {', '.join(keys)}")
    note("Input actions", "player.*",
         "Built in to the controller: player.forward/back/left/right (WASD), player.jump (Space), "
         "player.sprint (Shift), player.crouch (off: crouch_height 0).")


# ---------------------------------------------------------------- islands

CHEST = "debris/kenney-pirate/chest.glb"
PALMS = ["island/kenney-pirate/palm-straight.glb", "island/kenney-pirate/palm-bend.glb",
         "island/kenney-pirate/palm-detailed-straight.glb", "island/kenney-pirate/palm-detailed-bend.glb"]
ROCKS = ["island/kenney-pirate/rocks-a.glb", "island/kenney-pirate/rocks-b.glb", "island/kenney-pirate/rocks-c.glb"]
SAND_ROCKS = ["island/kenney-pirate/rocks-sand-a.glb", "island/kenney-pirate/rocks-sand-b.glb",
              "island/kenney-pirate/rocks-sand-c.glb"]
BUSHES = ["island/kenney-nature/plant_bush.glb", "island/kenney-nature/plant_bushDetailed.glb",
          "island/kenney-nature/plant_bushLarge.glb"]
SURVIVORS = ["character/kenney-mini/character-male-e.glb", "character/kenney-mini/character-female-e.glb",
             "character/kenney-mini/character-male-f.glb", "character/kenney-mini/character-female-f.glb",
             "character/kenney-mini/character-male-d.glb", "character/kenney-mini/character-female-b.glb"]


class Isle:
    def __init__(self, d):
        self.d, self.r = d, d["r"]
        self.s = snake(d["name"])
        self.root = ent(f"island.{self.s}", pos=(d["x"], 0, d["z"]))
        self.surf, self.count, self.keep = [], {}, []
        self.rng = random.Random(zlib.crc32(self.s.encode()))

    def n(self, thing):
        k = self.count.get(thing, 0) + 1
        self.count[thing] = k
        return f"{self.s}.{thing}.{k}"

    def world(self, x, z):
        return (r4(self.d["x"] + x), r4(self.d["z"] + z))

    def dome(self, x, z, r, top, color, sx=1.0, sz=1.0, yaw=0.0, ground=True, thing="ground", rough=0.95):
        """Flattened sphere: crosses the water line at radius r (times sx/sz), peak `top`."""
        a, b = 1.25 * r, top / 0.4  # x/a = 0.8 at the water line
        yc = top - b
        nm = self.n(thing)
        prim(nm, self.root, "Sphere", (x, yc, z), (2 * a * sx, 2 * b, 2 * a * sz), color, rot=(0, yaw, 0),
             rough=rough, collide="convex" if ground else None, ground=ground)
        if ground:
            self.surf.append(("dome", x, z, a * sx, a * sz, b, yc, yaw))
        return nm

    def flat(self, x, z, hx, hz, top):
        self.surf.append(("box", x, z, hx, hz, top))

    def h(self, x, z):
        best = -1e9
        for s in self.surf:
            if s[0] == "dome":
                _, cx, cz, ax, az, b, yc, yaw = s
                dx, dz = x - cx, z - cz
                c, sn = math.cos(yaw), math.sin(yaw)
                lx, lz = c * dx - sn * dz, sn * dx + c * dz
                q = (lx / ax) ** 2 + (lz / az) ** 2
                if q < 1:
                    best = max(best, yc + b * math.sqrt(1 - q))
            else:
                _, cx, cz, hx, hz, top = s
                if abs(x - cx) <= hx and abs(z - cz) <= hz:
                    best = max(best, top)
        return best

    def ground_y(self, x, z, foot=0.5):
        hs = [self.h(x + dx, z + dz) for dx, dz in ((0, 0), (foot, 0), (-foot, 0), (0, foot), (0, -foot))]
        y = min(hs)
        return -0.4 if y < -50 else y

    def reserve(self, x, z, rad):
        self.keep.append((x, z, rad))

    def free(self, x, z, rad):
        return all((x - a) ** 2 + (z - b) ** 2 >= (rad + c) ** 2 for a, b, c in self.keep)

    def prop(self, rel, thing, x, z, scale=1.0, yaw=None, sink=0.15, name=None, rot=None, **kw):
        yaw = self.rng.uniform(0, TAU) if yaw is None else yaw
        info = proto(rel)[1]
        sc = as3(scale)
        foot = 0.35 * max((info["max"][0] - info["min"][0]) * sc[0], (info["max"][2] - info["min"][2]) * sc[2])
        y = self.ground_y(x, z, foot) - sink
        nm = name or self.n(thing)
        model(rel, nm, self.root, (x, y, z), rot or (0, yaw, 0), scale, **kw)
        return nm

    def scatter(self, count, rmin, rmax, clear, place, max_h=None):
        placed = tries = 0
        while placed < count and tries < count * 40:
            tries += 1
            a = self.rng.uniform(0, TAU)
            dist = math.sqrt(self.rng.uniform(rmin ** 2, rmax ** 2))
            x, z = dist * math.cos(a), dist * math.sin(a)
            if not self.free(x, z, clear) or self.h(x, z) < 0.3:
                continue
            self.reserve(x, z, clear)
            place(x, z)
            placed += 1

    def palms(self, count, rmin, rmax, hmin=5.5, hmax=8.0, clear=2.5):
        def place(x, z):
            rel = self.rng.choice(PALMS)
            self.prop(rel, "palm", x, z, self.rng.uniform(hmin, hmax) / model_height(rel), sink=0.3)
        self.scatter(count, rmin, rmax, clear, place)

    def rocks(self, count, rmin, rmax, smin=0.5, smax=1.2, tint=None, clear=2.5, sandy=False, solid=False):
        def place(x, z):
            rel = self.rng.choice(SAND_ROCKS if sandy else ROCKS)
            self.prop(rel, "rock", x, z, self.rng.uniform(smin, smax), sink=0.4, tint=tint, solid=solid)
        self.scatter(count, rmin, rmax, clear, place)

    def bushes(self, count, rmin, rmax, smin=3.0, smax=5.0):
        def place(x, z):
            self.prop(self.rng.choice(BUSHES), "bush", x, z, self.rng.uniform(smin, smax), sink=0.1, shadows=False)
        self.scatter(count, rmin, rmax, 1.2, place)

    def reef(self, count, rmin, rmax, tint=CORAL):
        """Coral-tinted rocks around the shore, half under water, no colliders."""
        for i in range(count):
            a = i * TAU / count + self.rng.uniform(-0.2, 0.2)
            dist = self.rng.uniform(rmin, rmax)
            x, z = dist * math.cos(a), dist * math.sin(a)
            rel = self.rng.choice(ROCKS)
            model(rel, self.n("reef"), self.root, (x, -1.2 + self.rng.uniform(-0.3, 0.3), z),
                  (0, self.rng.uniform(0, TAU), 0), self.rng.uniform(0.5, 0.9), tint=tint, shadows=False)

    def chest(self):
        x, z = spot(self.d, "Chest")
        self.reserve(x, z, 2.0)
        yaw = math.atan2(x, z)  # Kenney chest front is +Z: face out to sea
        y = self.ground_y(x, z, 0.6) - 0.05
        for state in ("closed", "open"):
            nm = f"{self.s}.chest.{state}"
            model(CHEST, nm, self.root, (x, y, z), (0, yaw, 0), 0.75, visible=state == "closed",
                  anim="drop" if state == "closed" else "keep", autoplay="", classes=("chest",))
            if state == "open":
                E[f"{nm}.lid"]["transform"]["rotation"] = [-0.785, 0.0, 0.0]
        wx, wz = self.world(x, z)
        note("Islands", f"{self.s}.chest.closed / .open",
             f"Chest at world ({wx}, {r4(y)}, {wz}) (Spot::Chest). Closed is visible; on loot hide closed, show "
             "open (lid baked open; its animation has clips open/close/open-close, autoplay off).")

    def survivor(self, k):
        x, z = spot(self.d, "Survivor")
        self.reserve(x, z, 2.0)
        y = self.ground_y(x, z, 0.3) - 0.05
        nm = f"{self.s}.survivor"
        clips = human(SURVIVORS[k % len(SURVIVORS)], nm, self.root, (x, y, z), yaw_to(x, z), visible=False)
        wx, wz = self.world(x, z)
        note("Islands", nm, f"Survivor NPC (hidden) at world ({wx}, {r4(y)}, {wz}) (Spot::Survivor), facing "
                            f"the sea. Animation on the group, autoplay idle; clips {', '.join(clips[:12])}...")


def build_hook(I):
    I.dome(0, 0, 11, 1.6, SAND)
    I.dome(4.5, 2.5, 6.5, 1.25, SAND)
    I.dome(-5.5, 3.0, 5.0, 1.1, SAND)
    I.dome(-1.5, -2.5, 6.0, 1.9, GRASS, thing="grass")
    bx, bz = spot(I.d, "Barnaby")
    I.reserve(bx, bz, 2.5)
    # dock: from the south shore toward the raft (world z = -19)
    dock_x, z0, z1, top = 4.5, 7.5, 19.0, 0.7
    n_tiles = 5
    step = (z1 - z0) / n_tiles
    ent("hook_isle.dock", I.root, (dock_x, top, (z0 + z1) / 2))
    for i in range(n_tiles):
        zc = z0 + step * (i + 0.5) - (z0 + z1) / 2
        model("raft/kenney-pirate/platform.glb", f"hook_isle.dock.plank.{i + 1}", "hook_isle.dock",
              (0, -0.2, zc), (0, (i % 2) * PI, 0), (1.0, 0.85, step / 2.52 + 0.01), anchor="base")
    for i, zc in enumerate([z0 + 3.5, z0 + 7.5, z1 - 0.4]):
        for sx in (-1.15, 1.15):
            prim(f"hook_isle.dock.post.{i * 2 + (sx > 0) + 1}", "hook_isle.dock", "Cylinder",
                 (sx, -1.0, zc - (z0 + z1) / 2), (0.25, 2.3, 0.25), WOOD_DARK)
    collider("hook_isle.dock.collider", "hook_isle.dock", (0, -0.12, 0), (1.25, 0.12, (z1 - z0) / 2), ground=True)
    I.reserve(dock_x, 9.0, 2.0)
    note("Islands", "hook_isle.dock", f"Dock from the south shore toward the raft, world x {dock_x}, z "
                                      f"{-38 + z0}..{-38 + z1}, deck top y {top}; collider "
                                      "`hook_isle.dock.collider` (class ground).")
    # Barnaby, facing the raft at the origin
    wx, wz = I.world(bx, bz)
    by = I.ground_y(bx, bz, 0.4) - 0.05
    clips = human("character/pirate_captain.glb", "hook_isle.barnaby", I.root, (bx, by, bz),
                  yaw_to(-wx, -wz), height=1.85)
    note("Islands", "hook_isle.barnaby", f"Barnaby (pirate captain) at world ({wx}, {r4(by)}, {wz}) "
                                         f"(Spot::Barnaby), faces the raft. Animation on the group; clips "
                                         f"{', '.join(clips)}.")
    # hut, crates, palms, rocks
    I.reserve(-3.0, -3.5, 3.0)
    hut_y = I.ground_y(-3.0, -3.5, 1.5) - 0.1
    model("island/kenney-pirate/structure-roof.glb", "hook_isle.hut", I.root, (-3.0, hut_y, -3.5), (0, 0.5, 0), 1.1)
    collider("hook_isle.hut.collider", "hook_isle.hut", (0, 1.2, 0), (1.3, 1.2, 1.3))
    for i, (x, z, s) in enumerate([(3.2, 6.0, 0.9), (4.0, 4.9, 0.7), (0.4, 7.6, 0.75)]):
        I.prop("debris/kenney-pirate/crate.glb" if i != 1 else "debris/kenney-pirate/barrel.glb", "crate", x, z, s,
               sink=0.05)
        I.reserve(x, z, 1.0)
    I.prop("island/kenney-nature/sign.glb", "sign", 3.0, 8.6, 4.0, yaw=0.2, sink=0.1)
    I.palms(5, 2.0, 9.0, 6.0, 8.5)
    I.rocks(4, 8.5, 12.5, 0.35, 0.7, sandy=True)
    I.bushes(5, 1.0, 7.0)


def shrine(I, i):
    x, z = spot(I.d, "Shrine", i)
    I.reserve(x, z, 4.5)
    g = I.ground_y(x, z, 2.5)
    rock_h = 5.5
    nm = f"{I.s}.shrine.{i}"
    model("island/kenney-nature/cliff_block_rock.glb", f"{nm}.rock", I.root, (x, g - 0.6, z), (0, 0.8 + 0.3 * i, 0),
          (4.6, rock_h, 4.6), ground=True)
    ua = math.atan2(z, x)  # seaward; the ramp leaves on the inland side, so keep that side clear
    for k, da in enumerate((0.0, 1.1, -1.1, 2.0, -2.0)):  # boulders hide the block's flat sides
        a = ua + da
        model(ROCKS[k % 3], f"{nm}.boulder.{k + 1}", I.root, (x + 2.9 * math.cos(a), g - 0.5, z + 2.9 * math.sin(a)),
              (0, -a + 0.4 * k, 0), (0.8, 1.45 - 0.12 * k, 0.8))
    top = g - 0.6 + rock_h * 1.0
    ent(nm, I.root, (x, top, z))
    model("island/shrine_a.glb", f"{nm}.model", nm, (0, -0.05, 0), (0, yaw_to(-x, -z), 0), 1.0)
    prim(f"{nm}.glow", nm, "Icosahedron", (0, 2.9, 0), 0.45, (0.4, 0.9, 1.0), emissive=(1.2, 3.0, 3.6),
         visible=False, shadows=False)
    light(f"{nm}.light", nm, (0, 3.0, 0), (0.45, 0.85, 1.0), 900, 12, visible=False)
    # ramp from inland up to the pillar's inner face
    u = (x / math.hypot(x, z), z / math.hypot(x, z))
    rise = top - (g - 0.2)
    top_pt = (x - u[0] * 2.3, top - 0.02, z - u[1] * 2.3)
    run_ = rise * 2.0
    bot = (top_pt[0] - u[0] * run_, 0, top_pt[2] - u[1] * run_)
    bot = (bot[0], I.ground_y(bot[0], bot[2], 1.0) - 0.15, bot[2])
    ramp(f"{nm}.ramp", I.root, bot, top_pt, 2.2, 0.5, WOOD)
    wx, wz = I.world(x, z)
    note("Islands", nm, f"Tide shrine {i} (Spot::Shrine({i})) on a rock pillar, world ({wx}, {r4(top)}, {wz}); "
                        f"ramp `{nm}.ramp` climbs from inland. Activated look: show `{nm}.glow` and "
                        f"`{nm}.light` (both hidden).")


def build_palm_reef(I):
    I.dome(0, 0, 40, 2.5, SAND)
    I.dome(18, -14, 22, 2.0, SAND, sx=1.2)
    I.dome(-20, 12, 18, 1.8, SAND)
    I.dome(-4, -4, 24, 3.1, GRASS, sx=1.1, thing="grass")
    for i in range(I.d["shrines"]):
        shrine(I, i)
    I.chest()
    I.survivor(0)
    I.reserve(0, 0, 6)
    # tall silhouette: palm cluster around a big central rock
    I.prop("island/kenney-pirate/rocks-b.glb", "landmark_rock", 0, 0, 2.6, yaw=0.4, sink=0.6, solid=True)
    for k in range(5):
        a = k * TAU / 5 + 0.3
        x, z = 5.5 * math.cos(a), 5.5 * math.sin(a)
        rel = PALMS[k % 2]
        I.prop(rel, "tall_palm", x, z, I.rng.uniform(12.0, 14.5) / model_height(rel), sink=0.4)
    I.palms(16, 8, 36, 5.5, 9.0)
    I.bushes(14, 6, 34)
    I.rocks(8, 20, 42, 0.5, 1.1, sandy=True)
    I.reef(16, 44, 52)
    note("Islands", "palm_reef.landmark_rock", "Central rock (~9.5 m) ringed by 12-14 m palms: the far silhouette.")


def build_rusted_wreck(I):
    I.dome(0, 0, 55, 2.2, SAND_DARK)
    I.dome(-22, 18, 26, 1.8, SAND_DARK, sz=1.3)
    I.dome(25, 22, 22, 1.6, SAND_DARK)
    I.dome(-6, -2, 26, 2.7, (0.46, 0.5, 0.3), thing="grass")
    I.chest()
    I.survivor(1)
    gx, gz = spot(I.d, "Generator")
    I.reserve(gx, gz, 3.5)
    gy = I.ground_y(gx, gz, 1.2) - 0.05
    ent("rusted_wreck.generator", I.root, (gx, gy, gz), (0, yaw_to(-gx, -gz), 0))
    model("raft/kenney-factory/machine.glb", "rusted_wreck.generator.model", "rusted_wreck.generator", (0, 0, 0),
          (0, 0, 0), 1.4, tint=RUST_TINT, solid=True)
    prim("rusted_wreck.generator.glow", "rusted_wreck.generator", "Cube", (0, 1.25, -1.07), (0.6, 0.35, 0.05),
         (1.0, 0.8, 0.3), emissive=(3.0, 2.2, 0.6), visible=False, shadows=False)
    light("rusted_wreck.generator.light", "rusted_wreck.generator", (0, 2.4, -1.4), (1.0, 0.8, 0.45), 900, 12,
          visible=False)
    for k, (x, z) in enumerate([(1.8, 0.8), (2.4, -0.2)]):
        prim(f"rusted_wreck.generator.drum.{k + 1}", "rusted_wreck.generator", "Cylinder", (x, 0.45, z),
             (0.6, 0.9, 0.6), (0.42, 0.12, 0.08), rough=0.6, metal=0.3)
    wx, wz = I.world(gx, gz)
    note("Islands", "rusted_wreck.generator", f"Generator (Spot::Generator) at world ({wx}, {r4(gy)}, {wz}). "
                                             "Repaired look: show `rusted_wreck.generator.glow` and "
                                             "`rusted_wreck.generator.light`.")
    # beached freighter (blocking hull colliders)
    fx_, fz_ = 36.0, -33.0  # bow run aground on the south-east shore, stern in the water
    I.reserve(fx_, fz_, 14)
    ent("rusted_wreck.freighter", I.root, (fx_, -2.2, fz_), (0, 0.75, 0))
    model("island/kenney-watercraft/ship-cargo-a.glb", "rusted_wreck.freighter.model", "rusted_wreck.freighter",
          (0, 0, 0), (-0.07, 0, 0.3), 3.5, tint=(0.8, 0.5, 0.38), solid=True, anchor="y")
    # old mast wreck: the tall silhouette
    I.reserve(-24, -16, 7)
    I.prop("island/kenney-pirate/ship-wreck.glb", "mast_wreck", -24, -16, 2.0, yaw=-0.7, sink=1.4,
           rot=(-0.12, -0.7, 0.05), solid=True)
    I.reserve(10, 12, 4)
    I.prop("island/kenney-factory/crane.glb", "crane", 10, 12, 4.0, yaw=2.2, sink=0.2, tint=RUST_TINT)
    for k, (x, z, yaw) in enumerate([(-6, 22, 0.3), (-9.5, 24, 0.35), (16, -4, 1.4), (-14, -30, 2.0), (6, 30, 0.9)]):
        I.reserve(x, z, 3.5)
        I.prop("island/kenney-industrial/shipping-container-a.glb", "container", x, z, 6.0, yaw=yaw, sink=0.3,
               tint=[(1, .7, .55), (.8, .9, 1), (1, .85, .6)][k % 3], solid=True)
    I.rocks(12, 10, 50, 0.6, 1.3, tint=(0.8, 0.75, 0.7))
    I.palms(5, 15, 40, 5.0, 7.0)
    I.bushes(8, 6, 40)
    note("Islands", "rusted_wreck.freighter", "Beached cargo ship (~37 m, hull colliders, not ground); "
                                              "`rusted_wreck.mast_wreck.1` (~20 m mast) and the crane are the silhouette.")


def build_smoking_rig(I):
    g = "smoking_rig"
    top = 7.4
    prim(f"{g}.deck", I.root, "Cube", (0, top - 0.4, 0), (40, 0.8, 40), STEEL_DARK, rough=0.7, metal=0.3,
         collide="box", ground=True)
    I.flat(0, 0, 20, 20, top)
    for k, (x, z) in enumerate([(-15, -15), (15, -15), (-15, 15), (15, 15)]):
        prim(f"{g}.leg.{k + 1}", I.root, "Cylinder", (x, (top - 0.8 - 12) / 2, z), (2.5, top - 0.8 + 12, 2.5), RUST,
             rough=0.6, metal=0.4, collide="convex")
    for k, (a, b) in enumerate([((-15, -15), (15, -15)), ((-15, 15), (15, 15)), ((-15, -15), (-15, 15)),
                                ((15, -15), (15, 15))]):
        beam(f"{g}.brace.{k + 1}", I.root, (a[0], 2.0, a[1]), (b[0], 2.0, b[1]), 0.6, RUST, shape="Cube")
        beam(f"{g}.brace.{k + 5}", I.root, (a[0], 2.2, a[1]), (b[0], top - 0.9, b[1]), 0.45, RUST, shape="Cube")
    # access ramp on the +X side
    rz = 8.0
    ramp(f"{g}.ramp", I.root, (36.0, -0.6, rz), (20.0, top, rz), 3.0, 0.4, STEEL)
    for side in (-1.6, 1.6):
        beam(f"{g}.ramp.rail.{1 if side < 0 else 2}", I.root, (36.0, 0.4, rz + side), (20.0, top + 1.0, rz + side),
             0.12, (0.85, 0.65, 0.15), shape="Cube")
    I.reserve(28, rz, 3)
    # edge rails (gap where the ramp arrives)
    rails = [((-20, -19.9), (20, -19.9)), ((-20, 19.9), (20, 19.9)), ((-19.9, -20), (-19.9, 20)),
             ((19.9, -20), (19.9, rz - 1.6)), ((19.9, rz + 1.6), (19.9, 20))]
    for k, (a, b) in enumerate(rails):
        for lvl, hgt in ((0, 1.0), (1, 0.5)):
            beam(f"{g}.rail.{k * 2 + lvl + 1}", I.root, (a[0], top + hgt, a[1]), (b[0], top + hgt, b[1]), 0.08,
                 (0.85, 0.65, 0.15), shape="Cube")
        cx, cz = (a[0] + b[0]) / 2, (a[1] + b[1]) / 2
        hx, hz = max(abs(b[0] - a[0]) / 2, 0.1), max(abs(b[1] - a[1]) / 2, 0.1)
        collider(f"{g}.rail.collider.{k + 1}", I.root, (cx, top + 0.55, cz), (hx, 0.55, hz))
    # derrick: the tall silhouette (~28 m above the deck)
    dx_, dz_, hgt = -8.0, -8.0, 28.0
    I.reserve(dx_, dz_, 5)
    legs = [(-3, -3), (3, -3), (-3, 3), (3, 3)]
    for k, (x, z) in enumerate(legs):
        beam(f"{g}.derrick.leg.{k + 1}", I.root, (dx_ + x, top, dz_ + z), (dx_ + x * 0.2, top + hgt, dz_ + z * 0.2),
             0.35, (0.75, 0.32, 0.14), shape="Cube")
    for lv in range(1, 5):
        f = lv / 5
        y = top + hgt * f
        s = 3 * (1 - 0.8 * f)
        sq = [(-s, -s), (s, -s), (s, s), (-s, s)]
        for k in range(4):
            a, b = sq[k], sq[(k + 1) % 4]
            beam(f"{g}.derrick.bar.{lv * 4 + k - 3}", I.root, (dx_ + a[0], y, dz_ + a[1]), (dx_ + b[0], y, dz_ + b[1]),
                 0.2, (0.85, 0.85, 0.8), shape="Cube")
    prim(f"{g}.derrick.crown", I.root, "Cube", (dx_, top + hgt + 0.5, dz_), (1.8, 1.0, 1.8), (0.75, 0.32, 0.14))
    prim(f"{g}.derrick.floor", I.root, "Cube", (dx_, top + 0.15, dz_), (7.5, 0.3, 7.5), STEEL, collide="box",
         ground=True)
    # flare stack with fire, smoke and light
    sx_, sz_ = 14.0, -14.0
    I.reserve(sx_, sz_, 3)
    stack_h = 18.0
    prim(f"{g}.flare_stack", I.root, "Cylinder", (sx_, top + stack_h / 2, sz_), (1.2, stack_h, 1.2), STEEL,
         rough=0.6, metal=0.4, collide="convex")
    ft = top + stack_h + 0.2
    emitter(f"{g}.flare", I.root, (sx_, ft, sz_),
            fx("fire", rate=70.0, shape_size=[0.5, 0.5, 0.5], size=[1.2, 1.8], speed=[2.5, 4.0], lifetime=[0.6, 1.1],
               max_particles=300))
    emitter(f"{g}.smoke", I.root, (sx_, ft + 2.5, sz_),
            fx("smoke", rate=8.0, size=[2.0, 3.0], lifetime=[6.0, 9.0], speed=[2.0, 3.0], wind=[1.4, 0.0, 0.4],
               max_particles=120))
    light(f"{g}.flare.light", I.root, (sx_, ft + 1.0, sz_), (1.0, 0.55, 0.25), 3000, 25)
    # deck clutter
    I.reserve(9, 12, 4)
    I.prop("island/kenney-industrial/detail-tank-large.glb", "tank", 9, 12, 4.0, yaw=0.0, sink=0.0, solid=True)
    I.reserve(-14, 12, 3)
    I.prop("island/kenney-industrial/water-tower.glb", "water_tower", -14, 12, 3.0, yaw=0.0, sink=0.0)
    for k, (x, z, y_off) in enumerate([(6, -13, 0.0), (6, -8.5, 0.0), (6, -10.75, 2.1)]):
        I.reserve(x, z, 2.5)
        model("island/kenney-industrial/shipping-container-a.glb", I.n("container"), I.root, (x, top + y_off, z),
              (0, PI / 2, 0), 6.0, tint=[(1, .7, .55), (.8, .9, 1), (1, .85, .6)][k], solid=True)
    I.reserve(16, 16, 3)
    I.prop("island/kenney-factory/crane.glb", "crane", 16, 16, 3.0, yaw=-2.4, sink=0.0, tint=(1.0, 0.85, 0.5))
    I.reserve(-6, 12, 5)
    prim(f"{g}.helipad", I.root, "Cylinder", (-5, top + 0.03, 9), (9, 0.06, 9), (0.3, 0.32, 0.34), shadows=False)
    for k, (x, z, w, d) in enumerate([(-6.2, 9, 0.5, 3.2), (-3.8, 9, 0.5, 3.2), (-5, 9, 2.0, 0.5)]):
        prim(f"{g}.helipad.h.{k + 1}", I.root, "Cube", (x, top + 0.07, z), (w, 0.02, d), (0.95, 0.8, 0.2),
             shadows=False)
    I.chest()
    I.survivor(2)
    note("Islands", "smoking_rig.deck", f"Oil platform deck 40x40 m, top y {top}, class ground; reach it by "
                                        "`smoking_rig.ramp` on the +X side (z = 8).")
    note("Islands", "smoking_rig.flare / .smoke / .flare.light",
         "Burning flare stack (~25 m): fire and smoke emitters plus a visible orange point light.")


def build_lighthouse(I):
    I.dome(0, 0, 18, 3.5, ROCK, rough=0.9)
    I.dome(4, 6, 12, 3.0, SAND)
    I.dome(-2, -2, 11, 4.0, GRASS_DARK, thing="grass")
    # ring of reef islets (the atoll)
    for k in range(7):
        a = k * TAU / 7 + 0.25
        dist = I.rng.uniform(46, 54)
        x, z = dist * math.cos(a), dist * math.sin(a)
        rr = I.rng.uniform(4.5, 8.0)
        I.dome(x, z, rr, I.rng.uniform(0.9, 1.4), SAND, sx=I.rng.uniform(1.0, 1.8), yaw=a + PI / 2, thing="atoll")
        rel = I.rng.choice(PALMS if k % 2 else ROCKS)
        I.prop(rel, "atoll_prop", x, z, I.rng.uniform(5, 7) / model_height(rel) if k % 2 else I.rng.uniform(0.5, 0.8),
               sink=0.35)
    # lighthouse: tower axis sits at model (-1.77, -0.25), lantern ~10.2 m (unscaled)
    lx, lz, s = 0.0, -9.0, 2.0
    I.reserve(lx, lz, 6)
    ly = I.ground_y(lx, lz, 3.0) - 0.3
    ent("lighthouse_atoll.lighthouse", I.root, (lx, ly, lz))
    model("island/lighthouse.glb", "lighthouse_atoll.lighthouse.model", "lighthouse_atoll.lighthouse",
          (1.77 * s, 0, 0.25 * s), (0, 0, 0), s, anchor="origin", solid=True)
    lamp_y = 10.2 * s
    light("lighthouse_atoll.lamp", "lighthouse_atoll.lighthouse", (0, lamp_y, 0), (1.0, 0.85, 0.55), 5000, 60,
          visible=False)
    prim("lighthouse_atoll.lamp_glow", "lighthouse_atoll.lighthouse", "Sphere", (0, lamp_y, 0), 1.6,
         (1.0, 0.9, 0.6), emissive=(5.0, 4.2, 2.6), visible=False, shadows=False)
    # beacon socket at the centre (Spot::Beacon)
    bx, bz = spot(I.d, "Beacon")
    I.reserve(bx, bz, 3)
    by = I.ground_y(bx, bz, 1.2) - 0.1
    ent("lighthouse_atoll.beacon_socket", I.root, (bx, by, bz))
    prim("lighthouse_atoll.beacon_socket.base", "lighthouse_atoll.beacon_socket", "Cylinder", (0, 0.25, 0),
         (2.4, 0.5, 2.4), (0.6, 0.58, 0.55), collide="box", ground=True)
    for k in range(4):
        a = k * TAU / 4 + PI / 4
        model("island/kenney-nature/statue_column.glb", f"lighthouse_atoll.beacon_socket.column.{k + 1}",
              "lighthouse_atoll.beacon_socket", (0.85 * math.cos(a), 0.5, 0.85 * math.sin(a)), (0, a, 0),
              (2.0, 2.4, 2.0))
    prim("lighthouse_atoll.beacon_core", "lighthouse_atoll.beacon_socket", "Octahedron", (0, 1.6, 0), (0.6, 0.9, 0.6),
         (0.5, 0.9, 1.0), emissive=(2.0, 4.0, 5.0), visible=False, shadows=False)
    light("lighthouse_atoll.beacon_core.light", "lighthouse_atoll.beacon_socket", (0, 2.0, 0), (0.5, 0.85, 1.0),
          2500, 30, visible=False)
    I.rocks(8, 8, 17, 0.6, 1.2, solid=True)
    I.palms(4, 6, 14, 5, 7)
    I.bushes(6, 3, 12)
    wx, wz = I.world(lx, lz)
    note("Islands", "lighthouse_atoll.lighthouse", f"Lighthouse group at world ({wx}, {r4(ly)}, {wz}), ~26 m. "
                                                   f"Lamp: show `lighthouse_atoll.lamp` (point light at y +{lamp_y}) "
                                                   "and `lighthouse_atoll.lamp_glow` (both hidden).")
    note("Islands", "lighthouse_atoll.beacon_socket",
         f"Beacon socket (Spot::Beacon) at world ({r4(I.d['x'])}, {r4(by)}, {r4(I.d['z'])}). Installed core: show "
         "`lighthouse_atoll.beacon_core` and `lighthouse_atoll.beacon_core.light` (hidden).")


def build_islet(I, k):
    d, r = I.d, I.r
    s = I.s
    palm = d["style"] == "Palm"
    base = SAND if palm else (0.66, 0.62, 0.55)
    I.dome(0, 0, r, 1.2 + 0.05 * r, base)
    a = I.rng.uniform(0, TAU)
    I.dome(0.45 * r * math.cos(a), 0.45 * r * math.sin(a), 0.55 * r, 0.9 + 0.04 * r, base)
    if d["chest"]:
        I.chest()
    if d["survivor"]:
        I.survivor(k)
    if palm:
        I.dome(-0.1 * r, -0.15 * r, 0.55 * r, 1.45 + 0.06 * r, GRASS, thing="grass")
    special = {
        "gull_rock": lambda: (I.reserve(0, -2, 3), I.prop("island/kenney-nature/rock_tallA.glb", "spire", 0, -2,
                                                            (5, 11, 5), sink=0.4, tint=(1.15, 1.15, 1.12), solid=True)),
        "storm_spire": lambda: (I.reserve(-1, -1, 3), I.prop(ROCKS[1], "spire", -1, -1, (1.5, 6.0, 1.4), sink=0.4,
                                                             tint=(0.75, 0.75, 0.8), solid=True),
                                I.prop("island/kenney-nature/rock_tallB.glb", "spire", 2.5, 1.5, (3, 9, 3), sink=0.3,
                                       tint=(0.6, 0.6, 0.66), solid=True)),
        "barnacle_rock": lambda: (I.reserve(0, 0, 3), I.prop(ROCKS[1], "big_rock", 0, -1, 1.6, sink=0.5,
                                                             tint=(0.85, 0.85, 0.8), solid=True)),
        "sunken_bell": lambda: sunken_bell(I),
        "shellback": lambda: shellback(I),
        "ember_key": lambda: (I.rocks(5, 2, r * 0.8, 0.6, 1.0, tint=(0.75, 0.4, 0.32), solid=True),),
        "mangrove_isle": lambda: (I.bushes(14, 1, r * 0.85, 4, 7),),
        "driftwood_cay": lambda: [I.prop("debris/kenney-nature/log.glb", "driftwood", x, z, 5.0, sink=0.2)
                                  for x, z in ((2, -3), (-3, 2.5), (4.5, 2), (-1, -5.5))],
        "last_hope_cay": lambda: (I.reserve(-r * 0.6, -r * 0.55, 3),
                                  I.prop("boat/kenney-watercraft/boat-row-small.glb", "wreck_boat", -r * 0.6, -r * 0.55,
                                         1.3, rot=(0.3, 1.0, 0.5), sink=0.3)),
    }.get(s)
    if special:
        special()
    if palm:
        n_p = 2 + int(r // 3)
        I.palms(n_p, 1.0, r * 0.75, 4.5 if s == "mangrove_isle" else 5.5, 6.0 if s == "mangrove_isle" else 8.5)
        I.bushes(3 + int(r // 4), 1.0, r * 0.8)
    else:
        I.rocks(3 + int(r // 3), 1.0, r * 0.9, 0.45, 0.9, solid=True)
    I.rocks(2, r * 0.85, r * 1.1, 0.3, 0.55, sandy=True)


def sunken_bell(I):
    x, z = -2.0, -2.0
    I.reserve(x, z, 3.5)
    y = I.ground_y(x, z, 1.5)
    ent("sunken_bell.bell", I.root, (x, y, z), (0.0, 0.4, 0.25))
    for k, sx in enumerate((-1.5, 1.5)):
        model("island/kenney-nature/statue_column.glb", f"sunken_bell.bell.post.{k + 1}", "sunken_bell.bell",
              (sx, -0.2, 0), (0, 0, 0), (2.0, 4.0, 2.0))
    beam("sunken_bell.bell.beam", "sunken_bell.bell", (-1.7, 3.7, 0), (1.7, 3.7, 0), 0.4, WOOD_DARK, shape="Cube")
    prim("sunken_bell.bell.body", "sunken_bell.bell", "Cone", (0, 2.6, 0), (1.6, 1.8, 1.6), (0.6, 0.45, 0.2),
         rough=0.45, metal=0.8)
    prim("sunken_bell.bell.lip", "sunken_bell.bell", "Torus", (0, 1.75, 0), (1.7, 0.5, 1.7), (0.55, 0.42, 0.2),
         rough=0.45, metal=0.8)


def shellback(I):
    r = I.r
    I.dome(0, 0, r * 0.8, 3.4, (0.4, 0.44, 0.28), thing="shell", rough=0.8)
    prim("shellback.head", I.root, "Sphere", (0, 0.6, -r * 1.05), (3.2, 2.4, 3.8), (0.45, 0.55, 0.38))
    for k, (x, z) in enumerate([(-1, -1), (1, -1), (-1, 1), (1, 1)]):
        prim(f"shellback.flipper.{k + 1}", I.root, "Sphere", (x * r * 0.75, -0.1, z * r * 0.6), (4.5, 0.6, 2.2),
             (0.45, 0.55, 0.38), rot=(0, x * z * 0.6, 0))


def build_islands():
    isl = islands()
    styles = {d["style"] for d in isl}
    assert styles == {"Hook", "Palm", "Rock", "Wreck", "Rig", "Lighthouse"}, styles
    special = {"Hook": build_hook, "Wreck": build_rusted_wreck, "Rig": build_smoking_rig,
               "Lighthouse": build_lighthouse}
    k = 0
    for d in isl:
        I = Isle(d)
        if d["name"] == "Palm Reef":
            build_palm_reef(I)
        elif d["style"] in special:
            special[d["style"]](I)
        else:
            build_islet(I, k)
            k += 1
        note("Island roots", I.root, f"{d['name']}: world ({r4(d['x'])}, 0, {r4(d['z'])}), radius {d['r']}, "
                                     f"style {d['style']}, tier {d['tier']}. Children are named `{I.s}.*`.")
    note("Islands", "class ground",
         "Every walkable island collider (sand/grass domes, dock, rig deck, ramps, shrine rocks, beacon base) "
         "has class `ground` on the entity that owns the collider; props, hulls and rails do not.")


# ---------------------------------------------------------------- templates

def tpl(name, doc):
    ent(name, pos=(0, HIDE_Y, 0), visible=False)
    TPL_DOCS.append((name, doc))
    return name


TPL_DOCS = []


def raft_tile(name, y0):
    tpl(name, f"Raft floor tile 2x2 m, deck top at local y {y0}"
              + (" (place at y 0)." if y0 else " (place at cell_center y 0.45).")
              + f" Child `{name}.collider` is a Fixed box from the water line to the deck top.")
    model("raft/kenney-pirate/platform.glb", f"{name}.deck", name, (0, y0 - 0.184, 0), (0, 0, 0), (0.8, 0.8, 0.7937))
    for k, z in enumerate((-0.62, 0.0, 0.62)):
        prim(f"{name}.log.{k + 1}", name, "Cylinder", (0, y0 - 0.184 - 0.13, z), (0.32, 2.0, 0.32), WOOD_DARK,
             rot=(0, 0, PI / 2))
    collider(f"{name}.collider", name, (0, y0 - 0.225, 0), (1.0, 0.225, 1.0))


def build_raft_templates():
    raft_tile("tpl.raft_floor", 0.0)
    raft_tile("tpl.floor", DECK_Y)
    tpl("tpl.railing", "Railing for one tile edge: origin at the edge midpoint on the deck top, runs 2 m along "
                       "local X, ~1 m tall, Fixed box collider child. rot 0 = North/South edge; turn pi/2 for East/West.")
    model("raft/kenney-survival/fence.glb", "tpl.railing.fence", "tpl.railing", (0, 0, 0), (0, 0, 0), (4.0, 1.9, 4.0))
    collider("tpl.railing.collider", "tpl.railing", (0, 0.5, 0), (1.0, 0.5, 0.06))

    tpl("tpl.campfire", "Campfire (1 tile). Unlit; put a `tpl.fx.campfire` copy at (0, 0.15, 0) when lit.")
    model("raft/kenney-survival/campfire-pit.glb", "tpl.campfire.pit", "tpl.campfire", (0, 0, 0), (0, 0, 0), 3.5)
    model("raft/kenney-survival/campfire-stand.glb", "tpl.campfire.stand", "tpl.campfire", (0, 0, 0), (0, 0, 0), 3.5)

    tpl("tpl.workbench", "Workbench (1 tile), front -Z.")
    model("raft/kenney-survival/workbench.glb", "tpl.workbench.model", "tpl.workbench", (0, 0, 0), (0, PI, 0), 3.2)
    collider("tpl.workbench.collider", "tpl.workbench", (0, 0.46, 0), (0.53, 0.46, 0.48))

    tpl("tpl.logbook", "Logbook lectern (1 tile), book tilted toward -Z.")
    prim("tpl.logbook.stand", "tpl.logbook", "Cube", (0, 0.45, 0), (0.45, 0.9, 0.36), WOOD)
    model("raft/book_open.glb", "tpl.logbook.book", "tpl.logbook", (0, 0.9, 0), (-0.4, 0, 0), 0.7, anchor="center")

    tpl("tpl.shredder", "Shredder hopper (1 tile).")
    model("raft/kenney-factory/hopper-square.glb", "tpl.shredder.model", "tpl.shredder", (0, 0, 0), (0, 0, 0), 0.85)
    collider("tpl.shredder.collider", "tpl.shredder", (0, 0.43, 0), (0.48, 0.43, 0.48))

    tpl("tpl.wood_farm", "Bamboo planter, footprint 2x1 tiles: spans x -2..2, z -1..1 around the origin.")
    prim("tpl.wood_farm.box", "tpl.wood_farm", "Cube", (0, 0.175, 0), (3.8, 0.35, 1.8), WOOD, collide="box")
    prim("tpl.wood_farm.soil", "tpl.wood_farm", "Cube", (0, 0.36, 0), (3.6, 0.04, 1.6), (0.3, 0.2, 0.12),
         shadows=False)
    for k, x in enumerate((-1.35, -0.45, 0.45, 1.35)):
        for j, z in enumerate((-0.4, 0.4)):
            model("raft/kenney-nature/crops_bambooStageB.glb", f"tpl.wood_farm.crop.{k * 2 + j + 1}", "tpl.wood_farm",
                  (x + 0.12 * j, 0.36, z), (0, k + j, 0), 1.6, tint=(1.3, 1.0, 0.45))

    tpl("tpl.metal_farm", "Scrap press, footprint 2x1 tiles (x -2..2, z -1..1).")
    prim("tpl.metal_farm.base", "tpl.metal_farm", "Cube", (0, 0.15, 0), (3.8, 0.3, 1.8), STEEL, metal=0.4, rough=0.6,
         collide="box")
    model("raft/kenney-factory/machine.glb", "tpl.metal_farm.machine", "tpl.metal_farm", (-0.8, 0.3, 0), (0, PI / 2, 0),
          0.9)
    model("raft/kenney-factory/hopper-round.glb", "tpl.metal_farm.hopper", "tpl.metal_farm", (1.0, 0.3, 0), (0, 0, 0),
          0.75)

    tpl("tpl.oil_farm", "Oil still, footprint 2x1 tiles (x -2..2, z -1..1).")
    prim("tpl.oil_farm.base", "tpl.oil_farm", "Cube", (0, 0.15, 0), (3.8, 0.3, 1.8), STEEL_DARK, metal=0.4,
         rough=0.6, collide="box")
    model("raft/kenney-industrial/detail-tank.glb", "tpl.oil_farm.tank", "tpl.oil_farm", (-0.6, 0.3, 0), (0, 0, 0), 2.6)
    for k, z in enumerate((-0.45, 0.45)):
        prim(f"tpl.oil_farm.drum.{k + 1}", "tpl.oil_farm", "Cylinder", (1.35, 0.75, z), (0.6, 0.9, 0.6),
             (0.42, 0.1, 0.07), rough=0.5, metal=0.4)

    tpl("tpl.radio_station", "Radio station (1 tile), front -Z.")
    model("raft/radio_q.glb", "tpl.radio_station.model", "tpl.radio_station", (0, 0, 0), (0, PI, 0), 0.85)
    model("raft/antenna_b.glb", "tpl.radio_station.antenna", "tpl.radio_station", (0.5, 1.75, 0), (0, 0, 0), 1.3)
    collider("tpl.radio_station.collider", "tpl.radio_station", (0, 0.9, 0), (0.72, 0.9, 0.3))


def build_debris_templates():
    tpl("tpl.plank", "Floating driftwood planks; origin at the water line.")
    model("debris/kenney-survival/resource-planks.glb", "tpl.plank.model", "tpl.plank", (0, 0, 0), (0, 0, 0), 2.5,
          anchor="center")
    tpl("tpl.palm_bundle", "Floating palm leaves; origin at the water line.")
    model("debris/kenney-nature/grass_leafsLarge.glb", "tpl.palm_bundle.model", "tpl.palm_bundle", (0, -0.05, 0),
          (0, 0, 0), 3.5, tint=(0.8, 1.05, 0.7))
    model("debris/kenney-nature/log.glb", "tpl.palm_bundle.stick", "tpl.palm_bundle", (0, -0.02, 0), (0, 0.7, 0),
          (1.5, 1.5, 1.8), anchor="center")
    tpl("tpl.scrap_chunk", "Floating scrap metal; origin at the water line.")
    model("debris/kenney-survival/metal-panel-screws-half.glb", "tpl.scrap_chunk.panel", "tpl.scrap_chunk",
          (0, 0.0, 0), (-PI / 2, 0.4, 0), 1.8, anchor="center")
    model("debris/kenney-factory/cog-e.glb", "tpl.scrap_chunk.cog", "tpl.scrap_chunk", (0.25, 0.04, 0.1), (0.2, 0, 0),
          0.55, anchor="center")
    tpl("tpl.floating_crate", "Floating crate, a third under water; origin at the water line.")
    model("debris/kenney-pirate/crate.glb", "tpl.floating_crate.model", "tpl.floating_crate", (0, -0.3, 0),
          (0, 0, 0), 0.9)
    tpl("tpl.oil_barrel", "Floating oil drum lying on its side; origin at the water line.")
    prim("tpl.oil_barrel.drum", "tpl.oil_barrel", "Cylinder", (0, 0.05, 0), (0.6, 0.9, 0.6), (0.45, 0.08, 0.06),
         rot=(0, 0, PI / 2), rough=0.5, metal=0.4)
    for k, x in enumerate((-0.28, 0.28)):
        prim(f"tpl.oil_barrel.ring.{k + 1}", "tpl.oil_barrel", "Cylinder", (x, 0.05, 0), (0.63, 0.05, 0.63),
             (0.2, 0.2, 0.22), rot=(0, 0, PI / 2), metal=0.6, rough=0.5)
    tpl("tpl.crawler_carcass", "Floating crawler remains; origin at the water line.")
    model("debris/kenney-food/meat-ribs.glb", "tpl.crawler_carcass.ribs", "tpl.crawler_carcass", (0, -0.03, 0),
          (0, 0, 0), 2.6, tint=(0.75, 0.85, 0.7))
    model("debris/kenney-food/fish-bones.glb", "tpl.crawler_carcass.bones", "tpl.crawler_carcass", (0.3, -0.02, 0.3),
          (0, 1.0, 0), 1.6)
    tpl("tpl.gloomfin_carcass", "Floating gloomfin skeleton; origin at the water line.")
    model("debris/kenney-food/fish-bones.glb", "tpl.gloomfin_carcass.model", "tpl.gloomfin_carcass", (0, -0.05, 0),
          (0, 0, 0), 4.0, tint=(0.55, 0.5, 0.65))
    tpl("tpl.reef_crab_carcass", "Floating upturned reef crab; origin at the water line.")
    model("creature/crab.glb", "tpl.reef_crab_carcass.model", "tpl.reef_crab_carcass", (0, 0.15, 0), (PI, 0.5, 0), 0.4,
          anim="drop", tint=(1.0, 0.75, 0.65), anchor="center")
    tpl("tpl.brute_carcass", "Floating rig brute remains; origin at the water line.")
    model("debris/kenney-food/meat-ribs.glb", "tpl.brute_carcass.ribs", "tpl.brute_carcass", (0, -0.05, 0),
          (0, 0, 0), 4.5, tint=(0.9, 0.7, 0.6))
    model("debris/kenney-food/fish-bones.glb", "tpl.brute_carcass.bones", "tpl.brute_carcass", (-0.6, -0.03, 0.4),
          (0, 2.0, 0), 2.5, tint=(0.9, 0.8, 0.7))


def creature(name, rel, scale, doc, autoplay, anchor="base", tint=None, y=0.0):
    tpl(name, doc)
    E[name].pop("visible")
    E[name]["visible"] = False
    clips = model(rel, f"{name}.model", name, (0, y, 0), (0, PI, 0), scale, host=name, autoplay=autoplay,
                  anchor=anchor, tint=tint)
    TPL_DOCS[-1] = (name, doc + f" Animation on the root; clips: {', '.join(clips)}.")


def build_enemy_templates():
    creature("tpl.enemy.gloomfin", "creature/anglerfish.glb", 0.5,
             "Gloomfin (anglerfish ~2.7 m), forward -Z, origin at the body centre.", "swimming-normal", "center",
             tint=(0.7, 0.72, 0.9))
    creature("tpl.enemy.crawler", "creature/fish_generic_b.glb", 0.4,
             "Crawler (fish-man ~1.5 m), forward -Z, feet at the origin.", "idle", tint=(0.8, 0.95, 0.85))
    creature("tpl.enemy.reef_crab", "creature/crab.glb", 0.6, "Reef crab (~1.5 m wide), forward -Z, feet at the origin.",
             "idle", tint=(1.0, 0.75, 0.65))
    creature("tpl.enemy.rig_brute", "creature/giant.glb", 0.7,
             "Rig Brute miniboss (~2.8 m), forward -Z, feet at the origin. One instance: move this object "
             "directly (copies lose animation, see Notes).", "idle", tint=RUST_TINT)
    creature("tpl.enemy.leviathan", "creature/shark_a.glb", 1.2,
             "Leviathan night boss (shark ~19 m), forward -Z, origin at the body centre. One instance: move "
             "this object directly.", "swim", "center", tint=(0.45, 0.5, 0.62))


def boat(name, rel, scale, sink, doc, yaw=PI):
    tpl(name, doc)
    model(rel, f"{name}.model", name, (0, -sink, 0), (0, yaw, 0), scale)


def build_boat_templates():
    boat("tpl.boat.dinghy", "boat/kenney-watercraft/boat-row-small.glb", 1.3, 0.25,
         "Dinghy (2 seats), bow -Z, origin at the water line, centre of the hull.")
    boat("tpl.boat.skiff", "boat/kenney-watercraft/boat-fishing-small.glb", 1.3, 0.35,
         "Skiff (4 seats), bow -Z, origin at the water line.")
    boat("tpl.boat.motor_launch", "boat/kenney-watercraft/boat-speed-a.glb", 2.4, 0.5,
         "Motor Launch (8 seats, burns oil), bow -Z, origin at the water line.")
    tpl("tpl.trader_boat", "Trader houseboat with the trader NPC `tpl.trader_boat.trader` (animation on that "
                           "group). One instance: move/show this object directly (copies lose animation).")
    model("boat/kenney-watercraft/boat-house-a.glb", "tpl.trader_boat.model", "tpl.trader_boat", (0, -0.4, 0),
          (0, PI, 0), 1.8)
    clips = human("character/farmer.glb", "tpl.trader_boat.trader", "tpl.trader_boat", (0, 0.75, -2.7), 0.0)
    TPL_DOCS[-1] = (TPL_DOCS[-1][0], TPL_DOCS[-1][1] + f" Trader clips: {', '.join(clips)}.")


HELD = [
    ("harpoon", "item/spear_a.glb", 0.8, (0, PI, 0), "origin", None, None),
    ("club", "debris/kenney-nature/log.glb", (0.5, 0.5, 1.1), (0, 0, 0), "grip", (0.95, 0.85, 0.75), None),
    ("machete", "item/machete_ct.glb", 0.5, (0, 0, 0), "origin", None, None),
    ("cutlass", "item/sword_a.glb", 0.45, (-PI / 2, 0, 0), "grip", CORAL, None),
    ("abyss_blade", "item/sword_b.glb", 0.4, (-PI / 2, 0, 0), "grip", (0.4, 0.3, 0.6), (0.3, 0.1, 0.6)),
    ("rod_twig", "item/fishing_rod_a.glb", 0.3, (-PI / 2, 0, 0), "grip", (0.9, 0.75, 0.55), None),
    ("rod_reed", "item/fishing_rod_d.glb", 0.32, (-PI / 2, 0, 0), "grip", None, None),
    ("rod_carbon", "item/fishing_rod_c.glb", 0.3, (-PI / 2, 0, 0), "grip", (0.35, 0.35, 0.4), None),
    ("bandage", "item/first_aid_a.glb", 0.18, (0, 0, 0), "center", None, None),
    ("salve", "item/potion_a.glb", 0.25, (0, 0, 0), "center", (0.6, 1.0, 0.6), None),
    ("food", "item/kenney-food/meat-cooked.glb", 0.9, (0, 0, 0), "center", None, None),
    ("fish", "item/kenney-food/fish.glb", 0.9, (-PI / 2, 0, 0), "center", None, None),
    ("flare", "item/flare_gun.glb", 0.12, (0, PI / 2, 0), "origin", None, None),
    ("sac", "item/kenney-food/bag.glb", 0.9, (0, 0, 0), "center", (0.9, 0.8, 0.6), None),
]


def build_held_templates():
    for key, rel, s, rot, anchor, tint, emi in HELD:
        name = f"tpl.held.{key}"
        tpl(name, "")
        if anchor == "grip":  # bottom of the rotated model at the origin, so it extends toward -Z
            lo, hi = proto(rel)[1]["min"], proto(rel)[1]["max"]
            sc = as3(s)
            mn, mx = bbox([rot_apply(rot, (p[0] * sc[0], p[1] * sc[1], p[2] * sc[2])) for p in corners(lo, hi)])
            pos = (-(mn[0] + mx[0]) / 2, -(mn[1] + mx[1]) / 2, -mx[2] + 0.08)
            model(rel, f"{name}.model", name, pos, rot, s, anchor="origin", tint=tint, emissive=emi)
        else:
            model(rel, f"{name}.model", name, (0, 0, 0), rot, s, anchor=anchor, tint=tint, emissive=emi)
        lo, hi = bounds_of(name)
        tip = f" Tip at local z {r4(lo[2])}." if key.startswith(("rod", "harpoon", "cutlass", "abyss", "machete",
                                                                  "club")) else ""
        TPL_DOCS[-1] = (name, f"Held {key}: grip at the origin, points toward -Z (camera/hand forward).{tip}")


def build_misc_templates():
    tpl("tpl.harpoon_bolt", "Flying harpoon bolt (~1.2 m), centred, tip toward -Z.")
    model("item/spear_a.glb", "tpl.harpoon_bolt.model", "tpl.harpoon_bolt", (0, 0, 0), (0, PI, 0), 0.6,
          anchor="center")
    tpl("tpl.bobber", "Fishing bobber, origin at the water line.")
    prim("tpl.bobber.float", "tpl.bobber", "Sphere", (0, 0.0, 0), 0.16, (0.9, 0.12, 0.08), rough=0.4)
    prim("tpl.bobber.top", "tpl.bobber", "Sphere", (0, 0.07, 0), 0.1, WHITE, rough=0.4)
    tpl("tpl.rope", "Rope/line segment: 1 m along local Z centred on the origin, 3 cm thick. Place at the "
                    "midpoint, look_at one end, set_scale z = length.")
    prim("tpl.rope.line", "tpl.rope", "Cube", (0, 0, 0), (0.03, 0.03, 1.0), (0.85, 0.8, 0.65), shadows=False)
    tpl("tpl.ghost", "Build preview: semi-transparent emissive green box 2 x 0.2 x 2 centred on the origin. "
                     "set_color `tpl.ghost.box` copy red ([1,0.2,0.2,0.35]) when invalid. No collider.")
    prim("tpl.ghost.box", "tpl.ghost", "Cube", (0, 0, 0), (2.0, 0.2, 2.0), (0.2, 1.0, 0.3, 0.35),
         emissive=(0.1, 0.6, 0.15), shadows=False)
    tpl("tpl.fx.splash", "Water splash burst (one shot, plays when spawned; `trigger` replays).")
    emitter("tpl.fx.splash.burst", "tpl.fx.splash", (0, 0.05, 0), {
        **FX["sparks"], "blend": "Alpha", "bursts": [{"count": 40, "time": 0.0}], "looping": False,
        "color_over_life": [{"t": 0.0, "color": [0.9, 0.96, 1.0, 0.9]}, {"t": 1.0, "color": [0.7, 0.85, 0.95, 0.0]}],
        "emissive": 1.0, "facing": "Billboard", "gravity": 9.8, "lifetime": [0.5, 1.0], "size": [0.08, 0.2],
        "speed": [2.0, 4.5], "spread": 0.5, "sprite": "Soft", "stretch": 0.1, "max_particles": 80})
    tpl("tpl.fx.campfire", "Lit campfire: fire, embers and smoke emitters, plus point light "
                           "`tpl.fx.campfire.light` (flicker it with set_light). Place at campfire (0, 0.15, 0).")
    emitter("tpl.fx.campfire.fire", "tpl.fx.campfire", (0, 0.05, 0), fx("fire", rate=40.0, size=[0.3, 0.45]))
    emitter("tpl.fx.campfire.embers", "tpl.fx.campfire", (0, 0.2, 0), fx("embers"))
    emitter("tpl.fx.campfire.smoke", "tpl.fx.campfire", (0, 0.9, 0), fx("smoke", rate=6.0))
    light("tpl.fx.campfire.light", "tpl.fx.campfire", (0, 0.6, 0), (1.0, 0.6, 0.3), 600, 8)
    tpl("tpl.fx.hit", "Hit sparks burst (one shot, plays when spawned; `trigger` replays).")
    emitter("tpl.fx.hit.sparks", "tpl.fx.hit", (0, 0, 0), fx("sparks", bursts=[{"count": 25, "time": 0.0}],
                                                              speed=[2.0, 5.0], max_particles=60))


AVATARS = ["character/kenney-mini/character-female-a.glb", "character/kenney-mini/character-male-b.glb",
           "character/kenney-mini/character-female-b.glb", "character/kenney-mini/character-male-c.glb",
           "character/kenney-mini/character-female-c.glb", "character/kenney-mini/character-male-d.glb",
           "character/kenney-mini/character-female-d.glb"]


def build_avatars():
    tpl("tpl.avatar", "")
    clips = human("character/kenney-mini/character-male-a.glb", "tpl.avatar.body", "tpl.avatar", (0, 0, 0))
    TPL_DOCS[-1] = ("tpl.avatar", "Remote player body (1.8 m, feet at the origin, forward -Z). Copies lose "
                                  "animation; prefer the pool. Clips (on `tpl.avatar.body`): " + ", ".join(clips))
    for i, rel in enumerate(AVATARS, 1):
        human(rel, f"pool.avatar.{i}", None, (3.0 * i, HIDE_Y, 0), 0.0, visible=False)
    note("Pools", "pool.avatar.1 .. pool.avatar.7",
         "Pre-placed hidden remote-player bodies (different Kenney mini skins), feet at the origin, forward -Z, "
         "animation on the pool object itself (autoplay idle; same clips as Player Avatar). Assign one per "
         "remote player: set_position/set_rotation + set_visible + play_animation/crossfade. Named pool.* so "
         "they never clash with the view's `avatar.<player id>` copies.")


# ---------------------------------------------------------------- build

def build():
    build_look()
    build_player()
    build_islands()
    build_raft_templates()
    build_debris_templates()
    build_enemy_templates()
    build_boat_templates()
    build_held_templates()
    build_misc_templates()
    build_avatars()
    check_coverage()


def check_coverage():
    want = [f"tpl.{snake(v)}" for v in enumerate_variants("PlaceableKind")]
    want += [f"tpl.{snake(v)}" for v in enum_variants("ItemKind")[:9]]
    want += [f"tpl.enemy.{snake(v)}" for v in enum_variants("EnemyKind")]
    want += [f"tpl.boat.{snake(v)}" for v in enum_variants("BoatKind")]
    want += ["tpl.raft_floor", "tpl.floor", "tpl.railing", "tpl.avatar", "tpl.trader_boat"]
    missing = [n for n in want if n not in E]
    assert not missing, f"templates missing for content.rs variants: {missing}"


def enumerate_variants(name):
    return enum_variants(name)


def apply(scene_path, ents, scene_name, patch_path, extra_ops=()):
    scene_path.parent.mkdir(parents=True, exist_ok=True)
    scene_path.write_text(json.dumps(base_scene(scene_name), indent=2), encoding="utf-8")
    ops = [{"op": "create", "entity": e} for e in ents] + list(extra_ops)
    patch_path.write_text(json.dumps({"operations": ops}, separators=(",", ":")), encoding="utf-8")
    p = run([RUSTING, "scene", "patch", scene_path, patch_path, "--json"], check=False)
    try:
        res = json.loads(p.stdout)
    except ValueError:
        res = {"ok": False}
    if p.returncode != 0 or not res.get("ok"):
        sys.exit(f"scene patch failed:\n{p.stdout[-4000:]}\n{p.stderr[-2000:]}")


# ---------------------------------------------------------------- docs

def write_docs():
    lines = ["# Scene names (scenes/main.rscene)", "",
             "Generated by `python -I tools/scene/build_scene.py` from the scene it builds: do not edit by hand.",
             "Every name below exists in the scene. Axes: +Y up, forward is -Z for every template, NPC, enemy and "
             "boat (the player controller's convention); yaw 0 faces -Z. Units are metres.", "",
             f"Scene objects: {len(ENTS)}.", ""]
    order = ["Player and camera", "Input actions", "Look (lights, fog, post)", "Island roots", "Islands"]
    for sec in order:
        lines += [f"## {sec}", ""]
        for n, t in DOC.get(sec, []):
            lines.append(f"- `{n}`: {t}")
        lines.append("")
    lines += ["## Templates", "",
              "All templates are hidden roots at y -500 with identity rotation and scale 1 (copies keep the "
              "template's rotation and scale). Copy with spawn_copy_at_root, then set_position/set_rotation and "
              "set_visible(copy, true). Copied children are named `<copy>/<child name>`. Bounds are in the "
              "template's local space (min .. max).", "",
              "| Name | Bounds min .. max | Notes |", "| --- | --- | --- |"]
    for n, t in TPL_DOCS:
        lo, hi = bounds_of(n)
        lines.append(f"| `{n}` | {vec(lo)} .. {vec(hi)} | {t} |")
    lines.append("")
    for sec in ["Pools"]:
        lines += [f"## {sec}", ""]
        for n, t in DOC.get(sec, []):
            lines.append(f"- `{n}`: {t}")
        lines.append("")
    lines += ["## Notes for game code", "",
              "- Copies made with spawn_copy / spawn_copy_at_root do not animate and skinned meshes show their "
              "bind pose: the engine renames copied children `<copy>/<child>`, but animation track targets and "
              "skin joint paths still use the original child names. So animated things are used directly "
              "(`tpl.enemy.rig_brute`, `tpl.enemy.leviathan`, `tpl.trader_boat`, `pool.avatar.N`, survivors, "
              "Barnaby); `tpl.enemy.gloomfin/crawler/reef_crab` and `tpl.avatar` copies show a still rest pose "
              "until the engine copies fix the paths.",
              "- Hidden lights do not count toward the light cap (32 on Balanced). Visible point lights in the "
              "scene: `smoking_rig.flare.light` only.",
              "- No looping ambience in the scene: start it from code with "
              "`play_sound_looped(\"sfx/ocean_loop.wav\", vol)` and `sfx/wind_loop.wav` (also "
              "`water_lap_loop.ogg`, `fire_loop.ogg` near campfires).",
              "- The ocean surface is not in this scene (the view module owns it). Nothing here is named Ocean or "
              "Water.",
              "- Shadow quality/reach is code-only (rusting.render_settings): Auto or Medium is a good default.",
              "- Name tags for remote players: a rusting.hud with `follow` set to the avatar object, or egui.",
              ""]
    DOCS.write_text("\n".join(lines), encoding="utf-8")


# ---------------------------------------------------------------- capture views

def dup(root, new_root, pos, visible=True):
    """Copy a subtree under new names (capture scenes only)."""
    out = []
    names = subtree(root)
    ren = {n: new_root + n[len(root):] if n.startswith(root) else f"{new_root}/{n}" for n in names}
    for n in names:
        e = copy.deepcopy(E[n])
        e["name"] = ren[n]
        e["id"] = str(uuid.uuid5(NS, "capture:" + ren[n]))
        if n == root:
            e.pop("parent", None)
            e["transform"]["position"] = vec(pos)
            if visible:
                e.pop("visible", None)
        else:
            e["parent"] = ren[E[n]["parent"]]
        out.append(e)
    return out


CAP_PREFIX = r"..\..\assets" + "\\"  # asset paths seen from build/capture/


def capture(views):
    """Capture scenes start from the built main.rscene (engine format, so no slow re-patch of
    ~1400 objects); only the extra capture objects go through `scene patch`."""
    CAPTURE.mkdir(parents=True, exist_ok=True)
    main_doc = json.loads(SCENE.read_text(encoding="utf-8"))
    ents = fix_paths(main_doc["entities"], CAP_PREFIX)
    byname = {e["name"]: e for e in ents}
    extra = []

    def water(name, x, z, size):
        extra.append({"id": str(uuid.uuid5(NS, "capture:" + name)), "name": name, "transform": {
            "position": vec((x, 0.0, z)), "rotation": [0, 0, 0], "scale": [1, 1, 1]},
            "components": {"rusting.water": {"size": [size, size], "resolution": 128, "wave_height": 0.2,
                                             "wave_length": 6.0, "wave_speed": 1.0,
                                             "color": [0.1, 0.35, 0.5, 0.85]}}})
    water("Capture Water", 0, -20, 260)
    for d in islands():
        if d["name"] != "Hook Isle":
            water(f"Capture Water {snake(d['name'])}", d["x"], d["z"], max(140, d["r"] * 3.2))
    # fake 4x4 start raft with a few structures
    for i in range(-2, 2):
        for j in range(-2, 2):
            extra += dup("tpl.raft_floor", f"cap.tile.{i}.{j}", ((i + 0.5) * 2, DECK_Y, (j + 0.5) * 2))
    for n, pos in [("tpl.campfire", (1, DECK_Y, 1)), ("tpl.workbench", (-3, DECK_Y, -1)),
                   ("tpl.fx.campfire", (1, DECK_Y + 0.15, 1))]:
        extra += dup(n, "cap." + n, pos)
    for k in range(4):
        extra += dup("tpl.railing", f"cap.rail.{k}", ((k - 1.5) * 2, DECK_Y, 4))
    # lineup of templates on a sand slab at z ~ 200
    lz = 200.0
    extra.append({"id": str(uuid.uuid5(NS, "capture:slab")), "name": "Capture Slab",
                  "transform": {"position": [0, -0.05, lz + 15], "rotation": [0, 0, 0], "scale": [140, 0.1, 50]},
                  "mesh_renderer": {"mesh": {"BuiltinPrimitive": "Cube"}, "material": mat(SAND),
                                    "cast_shadows": True, "receive_shadows": True}})
    rows = [
        ["tpl.raft_floor", "tpl.floor", "tpl.railing", "tpl.campfire", "tpl.workbench", "tpl.logbook", "tpl.shredder",
         "tpl.wood_farm", "tpl.metal_farm", "tpl.oil_farm", "tpl.radio_station", "tpl.ghost", "tpl.fx.campfire"],
        ["tpl.plank", "tpl.palm_bundle", "tpl.scrap_chunk", "tpl.floating_crate", "tpl.oil_barrel",
         "tpl.crawler_carcass", "tpl.gloomfin_carcass", "tpl.reef_crab_carcass", "tpl.brute_carcass",
         "tpl.harpoon_bolt", "tpl.bobber", "tpl.rope"] + [f"tpl.held.{h[0]}" for h in HELD],
        ["tpl.avatar", "pool.avatar.1", "pool.avatar.2", "tpl.enemy.crawler", "tpl.enemy.reef_crab",
         "tpl.enemy.rig_brute", "tpl.enemy.gloomfin", "tpl.trader_boat"],
        ["tpl.boat.dinghy", "tpl.boat.skiff", "tpl.boat.motor_launch", "tpl.enemy.leviathan"],
    ]
    row_z = [lz, lz + 8, lz + 18, lz + 32]
    placed = []  # (row, name, centre x, z, width, height) for the close-up shots
    for r_i, (row, z) in enumerate(zip(rows, row_z)):
        widths = []
        for n in row:
            lo, hi = bounds_of(n)
            widths.append(max(hi[0] - lo[0], 0.5))
        gap = 1.2 if z == lz + 8 else 2.0
        x = -(sum(widths) + gap * (len(row) - 1)) / 2
        for n, w in zip(row, widths):
            lo, hi = bounds_of(n)
            e = byname[n]
            y = max(0.0, -lo[1]) if z != lz + 8 else 0.25 - lo[1]
            e["transform"]["position"] = vec((x - lo[0], y, z))
            e["visible"] = None
            placed.append((r_i, n, x + w / 2, z, w, y + hi[1]))
            x += w + gap
        # 2 m tile and 1.8 m human references at the row end
        for k, (sc, col) in enumerate((((2, 0.1, 2), (0.3, 0.3, 0.9)), ((0.4, 1.8, 0.4), (0.9, 0.2, 0.2)))):
            extra.append({"id": str(uuid.uuid5(NS, f"capture:ref{z}{k}")), "name": f"Capture Ref {z} {k}",
                          "transform": {"position": vec((x + 1.5 + 2.5 * k, sc[1] / 2, z)), "rotation": [0, 0, 0],
                                        "scale": list(sc)},
                          "mesh_renderer": {"mesh": {"BuiltinPrimitive": "Cube"}, "material": mat(col),
                                            "cast_shadows": True, "receive_shadows": True}})
    # survivors and island lights on for the capture
    for e in ents:
        if e["name"].endswith((".survivor", ".glow", ".light", ".lamp", ".lamp_glow", ".beacon_core")) and                 not e["name"].startswith(("tpl.", "pool.")):
            e["visible"] = None
    apply(CAPTURE / "extra.rscene", fix_paths(extra, CAP_PREFIX), "Capture Extra",
          CAPTURE / "extra.patch.json")
    extra_eng = json.loads((CAPTURE / "extra.rscene").read_text(encoding="utf-8"))["entities"]
    scene = CAPTURE / "world.rscene"
    scene.write_text(json.dumps(dict(main_doc, name="Capture World", entities=ents + extra_eng)), encoding="utf-8")
    # player drop test above Hook Isle: the active camera shows whether the dome holds him
    for e in ents:
        if e["name"] == "Player":
            e["transform"]["position"] = [-2.0, 6.0, -38.0]
            pc = json.loads(e["components"]["rusting.player_controller"])
            pc["yaw"] = 1.2
            e["components"]["rusting.player_controller"] = json.dumps(pc)
    water_eng = [e for e in extra_eng if e["name"] == "Capture Water"]
    (CAPTURE / "drop.rscene").write_text(json.dumps(dict(main_doc, name="Capture Drop", entities=ents + water_eng)),
                                         encoding="utf-8")
    isl = {snake(d["name"]): d for d in islands()}

    def around(key, dist, height, ang=0.8, look_h=3.0):
        d = isl[key]
        return (f"{d['x'] + dist * math.sin(ang):.1f},{height:.1f},{d['z'] + dist * math.cos(ang):.1f}",
                f"{d['x']:.1f},{look_h:.1f},{d['z']:.1f}")

    shots = {
        "origin": ("6,4,10", "0,2,-30"),
        "origin_wide": ("0,3,6", "0,3,-60"),
        "hook": ("16,7,-16", "0,1.5,-36"),
        "hook_back": ("-14,6,-58", "0,1.5,-38"),
        "palm_reef": around("palm_reef", 90, 30),
        "palm_reef_shrine": (f"{-420 + 28 - 14},{12},{-560 + 10}", f"{-420 + 28},{6},{-560}"),
        "rusted_wreck": around("rusted_wreck", 120, 35, ang=2.4),
        "smoking_rig": around("smoking_rig", 90, 30, ang=1.2, look_h=10),
        "lighthouse": around("lighthouse_atoll", 110, 30, ang=0.5, look_h=8),
        "storm_spire": around("storm_spire", 40, 10, look_h=6),
        "shellback": around("shellback", 35, 9),
        "sunken_bell": around("sunken_bell", 30, 8),
        "mangrove": around("mangrove_isle", 40, 12),
        "row_raft": (f"0,6,{lz - 14}", f"0,0.5,{lz}"),
        "row_small": (f"0,3.5,{lz + 1}", f"0,0.3,{lz + 8}"),
        "row_chars": (f"0,5,{lz + 7}", f"0,1.2,{lz + 18}"),
        "row_boats": (f"0,10,{lz + 12}", f"0,1.5,{lz + 32}"),
        "far_from_raft": ("0,3,0", f"{250},{2},{-180}"),
        "lighthouse_top": (f"{-2100 + 14},{24},{1700 - 9 + 14}", f"{-2100},{19},{1700 - 9}"),
        "wreck_close": (f"{1150 + 10},{8},{-600 - 4}", f"{1150},{2},{-600 - 16.5}"),
        "freighter": (f"{1150 + 75},{14},{-600 - 75}", f"{1150 + 36},{3},{-600 - 33}"),
        "barnaby": ("6,2.5,-27", "2,1.5,-31"),
        "rig_deck": (f"{300 + 26},{15},{1950 + 22}", f"{300},{8},{1950}"),
    }
    # template close-ups: consecutive lineup items in groups about 9 m wide
    group, k = [], 0
    for item in placed + [None]:
        if group and (item is None or item[0] != group[0][0] or item[2] + item[4] / 2 - (group[0][2] - group[0][4] / 2) > 9):
            x0 = group[0][2] - group[0][4] / 2
            x1 = group[-1][2] + group[-1][4] / 2
            h = max(g[5] for g in group)
            cx, z = (x0 + x1) / 2, group[0][3]
            dist = max(2.2, 0.75 * (x1 - x0) + 0.6 * h)
            shots[f"tpl_{k}"] = (f"{cx:.2f},{h * 0.6 + dist * 0.35:.2f},{z - dist:.2f}", f"{cx:.2f},{h * 0.4:.2f},{z:.2f}")
            print(f"tpl_{k}: " + ", ".join(g[1] for g in reversed(group)) + " (left to right)")
            k += 1
            group = []
        if item is not None:
            group.append(item)
    by_name = {g[1]: g for g in placed}
    tb = by_name["tpl.trader_boat"]
    shots["trader_back"] = (f"{tb[2] + 3:.2f},{tb[5] + 3:.2f},{tb[3] + 10:.2f}", f"{tb[2]:.2f},1.5,{tb[3]:.2f}")
    held = [g for g in placed if g[1].startswith("tpl.held.")]
    hx = sum(g[2] for g in held) / len(held)
    shots["held_top"] = (f"{hx:.2f},5.5,{held[0][3] - 2.5:.2f}", f"{hx:.2f},0,{held[0][3]:.2f}")
    shots["chest_palm"] = ("-403.15,3.6,-547.97", "-406,2.4,-550")
    todo = views or list(shots) + ["drop"]
    for v in todo:
        out = CAPTURE / f"{v}.png"
        if v == "drop":
            cmd = [RUSTING, "capture", CAPTURE / "drop.rscene", out, "--tick", "150", "--size", "1280x720", "--no-hud",
                   "--json"]
        else:
            at, look = shots[v]
            cmd = [RUSTING, "capture", scene, out, "--at", at, "--look-at", look, "--tick", "30", "--size",
                   "1280x720", "--no-hud", "--json"]
        p = run(cmd, check=False)
        try:
            res = json.loads(p.stdout)
            msg = "ok" if res.get("ok") else json.dumps(res.get("diagnostics"))[:400]
            rnd = (res.get("data") or {}).get("render") or {}
        except ValueError:
            msg, rnd = p.stdout[-400:] + p.stderr[-400:], {}
        print(f"capture {v}: {msg} {out} dropped_lights={rnd.get('dropped_lights')}")


def main(argv):
    PROTOS.mkdir(parents=True, exist_ok=True)
    build()
    assert len(ENTS) < 2500, f"{len(ENTS)} scene objects (budget 2500)"
    if "--capture-only" not in argv:  # --capture-only reuses the scene built last time
        apply(SCENE, ENTS, "RustingRaft Main Scene", WORK / "main.patch.json")
        write_docs()
        print(f"{SCENE.relative_to(ROOT)}: {len(ENTS)} objects; {DOCS.relative_to(ROOT)} written")
    if "--capture" in argv or "--capture-only" in argv:
        capture([a for a in argv if not a.startswith("--")])


if __name__ == "__main__":
    main(sys.argv[1:])
