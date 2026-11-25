# Renzora Live Link: streams this Blender scene to the Renzora editor.
#
# Installed for you: the editor's Blender Live Link plugin copies this file
# into every Blender 4.2+ user's scripts/addons folder when the editor starts
# and enables it in Blender's preferences (or, with Blender open, asks you to
# tick it in Edit > Preferences > Add-ons, which loads it at once). A newer
# copy written later reloads itself in a running Blender. In the 3D
# Viewport's sidebar (N), open the Renzora tab and press Connect while the
# editor is running.
#
# What it sends, and when:
#   - every object's transform and visibility, checked 30 times a second;
#   - a mesh object's evaluated geometry (all modifiers applied) when
#     Blender reports its geometry changed, at most 10 times a second;
#   - a material (Principled BSDF factors, base colour and normal textures)
#     when it changes;
#   - lights (point, spot, sun; area as point);
#   - renames and deletions.
# Everything is converted to the engine's frame first: Y up, glTF texture
# coordinates.
#
# Wire format: each frame is a little-endian u32 header length, u32 payload
# length, a JSON header, then raw payload bytes (float32 / uint32 arrays and
# image files). See the plugin's src/protocol.rs.

bl_info = {
    "name": "Renzora Live Link",
    "author": "Renzora",
    "version": (1, 0, 0),
    "blender": (4, 2, 0),
    "location": "3D Viewport > Sidebar > Renzora",
    "description": "Live link to the Renzora editor: objects, meshes, materials and lights update as you work",
    "category": "Import-Export",
}

import json
import math
import os
import queue
import socket
import struct
import threading
import time

import bpy
import numpy as np
from mathutils import Matrix

DEFAULT_PORT = 47810
TICK = 1.0 / 30.0
MESH_EVERY = 0.1

# Blender is Z up, the engine Y up: (x, y, z) -> (x, z, -y). A rotation, so
# triangle winding survives it.
AXIS = Matrix(((1, 0, 0, 0), (0, 0, 1, 0), (0, -1, 0, 0), (0, 0, 0, 1)))
AXIS_INV = AXIS.transposed()
# A Blender light shines down its local -Z, which the axis change turns into
# local -Y; an engine light shines down its local -Z. This turns one into the
# other.
LIGHT_FIX = Matrix.Rotation(-math.pi / 2, 4, "X")

MESH_TYPES = {"MESH", "CURVE", "SURFACE", "FONT", "META"}


# ---------------------------------------------------------------------------
# Connection
# ---------------------------------------------------------------------------


class Connection:
    """A socket with a writer thread, so a large mesh never blocks Blender's
    UI while it goes out, and a reader thread for the editor's replies."""

    def __init__(self, host, port):
        self.sock = socket.create_connection((host, port), timeout=3)
        self.sock.settimeout(None)
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.out = queue.Queue()
        self.alive = True
        self.why = ""
        self.project = ""
        threading.Thread(target=self._write, daemon=True).start()
        threading.Thread(target=self._read, daemon=True).start()

    def send(self, header, payload=b""):
        if not self.alive:
            return
        body = json.dumps(header).encode("utf-8")
        self.out.put(struct.pack("<II", len(body), len(payload)) + body)
        if payload:
            self.out.put(payload)

    def _write(self):
        while self.alive:
            chunk = self.out.get()
            if chunk is None:
                break
            try:
                self.sock.sendall(chunk)
            except OSError as e:
                self._fail("connection lost (%s)" % e)
                break

    def _read(self):
        try:
            while self.alive:
                lens = self._exact(8)
                header_len, payload_len = struct.unpack("<II", lens)
                header = json.loads(self._exact(header_len).decode("utf-8"))
                if payload_len:
                    self._exact(payload_len)
                if header.get("type") == "welcome":
                    self.project = header.get("project", "")
        except (OSError, ValueError) as e:
            self._fail(str(e) or "the editor closed the link")

    def _exact(self, n):
        data = bytearray()
        while len(data) < n:
            chunk = self.sock.recv(n - len(data))
            if not chunk:
                raise OSError("the editor closed the link")
            data += chunk
        return bytes(data)

    def _fail(self, why):
        if self.alive:
            self.alive = False
            self.why = why

    def close(self):
        self.alive = False
        self.out.put(None)
        try:
            self.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.sock.close()


# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------


class State:
    def __init__(self):
        self.conn = None
        self.status = "Not connected"
        self.reset()

    def reset(self):
        # Per object name: what was last sent, to send only changes.
        self.objects = {}
        # Pointer to name, to tell a rename from a delete plus an add.
        self.names = {}
        self.dirty_meshes = set()
        self.dirty_materials = set()
        self.mesh_sent_at = {}
        self.material_sigs = {}
        self.sent_textures = set()
        self.warned = set()


STATE = State()


def connected():
    return STATE.conn is not None and STATE.conn.alive


# ---------------------------------------------------------------------------
# What gets sent
# ---------------------------------------------------------------------------


def kind_of(ob):
    if ob.type in MESH_TYPES:
        return "mesh"
    if ob.type == "LIGHT":
        return "light"
    return "empty"


def transform_of(ob, kind):
    m = AXIS @ ob.matrix_world @ AXIS_INV
    if kind == "light":
        m = m @ LIGHT_FIX
    loc, rot, scale = m.decompose()
    return [loc.x, loc.y, loc.z], [rot.x, rot.y, rot.z, rot.w], [scale.x, scale.y, scale.z]


def send_object(ob, kind):
    t, r, s = transform_of(ob, kind)
    STATE.conn.send({
        "type": "object",
        "name": ob.name,
        "kind": kind,
        "translation": t,
        "rotation": r,
        "scale": s,
        "visible": ob.visible_get(),
    })


def light_header(ob):
    light = ob.data
    header = {
        "type": "light",
        "object": ob.name,
        "light": light.type.lower(),
        "color": list(light.color),
        "power": light.energy,
        "radius": getattr(light, "shadow_soft_size", 0.1),
        "shadows": light.use_shadow,
    }
    if light.type == "SPOT":
        header["spot_size"] = light.spot_size
        header["spot_blend"] = light.spot_blend
    if getattr(light, "use_custom_distance", False):
        header["range"] = light.cutoff_distance
    return header


def socket_by_id(node, identifier):
    # Sockets by identifier, never by display name: names are translated in a
    # localised Blender.
    for s in node.inputs:
        if s.identifier == identifier:
            return s
    return None


def upstream(sock, node_type):
    if sock is None or not sock.is_linked:
        return None
    node = sock.links[0].from_node
    return node if node.type == node_type else None


def texture_entry(image, payload):
    """The `textures` entry for an image, appending its bytes to `payload`
    the first time this connection sends it."""
    if image is None:
        return None
    if image.packed_file:
        data = bytes(image.packed_file.data)
        stamp = "packed-%d" % len(data)
        path = image.name
    else:
        path = bpy.path.abspath(image.filepath, library=image.library)
        if not os.path.isfile(path):
            warn_once("missing:" + image.name, "image %s has no file on disk; skipped" % image.name)
            return None
        stamp = "%d" % os.path.getmtime(path)
        data = None
    lower = path.lower()
    if lower.endswith(".png") or image.file_format == "PNG":
        mime = "image/png"
    elif lower.endswith((".jpg", ".jpeg")) or image.file_format == "JPEG":
        mime = "image/jpeg"
    else:
        warn_once("format:" + image.name, "image %s is not PNG or JPEG; skipped" % image.name)
        return None
    key = "%s|%s|%s" % (image.name, path, stamp)
    entry = {"key": key, "mime": mime}
    if key not in STATE.sent_textures:
        if data is None:
            with open(path, "rb") as f:
                data = f.read()
        entry["offset"] = len(payload)
        entry["length"] = len(data)
        payload += data
        STATE.sent_textures.add(key)
    return entry


def uses_nodes(mat):
    # `use_nodes` is deprecated in Blender 5 (every material uses nodes) and
    # goes in 6; read it only where it still exists.
    if mat.node_tree is None:
        return False
    if bpy.app.version >= (5, 0, 0):
        return True
    return mat.use_nodes


def material_message(mat):
    header = {
        "type": "material",
        "name": mat.name,
        "base_color": list(mat.diffuse_color),
        "metallic": mat.metallic,
        "roughness": mat.roughness,
        "double_sided": not mat.use_backface_culling,
        "blend": False,
    }
    payload = bytearray()
    textures = {}
    bsdf = None
    if uses_nodes(mat):
        bsdf = next((n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED"), None)
    if bsdf is not None:
        base = socket_by_id(bsdf, "Base Color")
        alpha = socket_by_id(bsdf, "Alpha")
        image_node = upstream(base, "TEX_IMAGE")
        a = alpha.default_value if alpha is not None else 1.0
        if image_node is not None:
            entry = texture_entry(image_node.image, payload)
            if entry:
                textures["base_color"] = entry
            header["base_color"] = [1.0, 1.0, 1.0, a]
        elif base is not None:
            c = base.default_value
            header["base_color"] = [c[0], c[1], c[2], a]
        header["blend"] = a < 1.0 or (alpha is not None and alpha.is_linked)
        for key, ident in (("metallic", "Metallic"), ("roughness", "Roughness")):
            s = socket_by_id(bsdf, ident)
            if s is not None:
                header[key] = s.default_value
        emission = socket_by_id(bsdf, "Emission Color")
        strength = socket_by_id(bsdf, "Emission Strength")
        if emission is not None and strength is not None:
            header["emissive"] = list(emission.default_value)[:3]
            header["emissive_strength"] = strength.default_value
        normal_map = upstream(socket_by_id(bsdf, "Normal"), "NORMAL_MAP")
        if normal_map is not None:
            image_node = upstream(socket_by_id(normal_map, "Color"), "TEX_IMAGE")
            if image_node is not None:
                entry = texture_entry(image_node.image, payload)
                if entry:
                    textures["normal"] = entry
                    s = socket_by_id(normal_map, "Strength")
                    header["normal_scale"] = s.default_value if s is not None else 1.0
    if textures:
        header["textures"] = textures
    return header, bytes(payload)


def material_signature(mat):
    """Everything send_material reads, cheaply, to skip unchanged materials."""
    parts = [tuple(mat.diffuse_color), mat.metallic, mat.roughness, mat.use_backface_culling]
    if uses_nodes(mat):
        for node in mat.node_tree.nodes:
            parts.append(node.name)
            if node.type == "TEX_IMAGE" and node.image is not None:
                parts.append((node.image.name, node.image.filepath))
            for s in node.inputs:
                if s.is_linked:
                    parts.append((s.identifier, s.links[0].from_node.name))
                elif hasattr(s, "default_value"):
                    v = s.default_value
                    parts.append(tuple(v) if hasattr(v, "__len__") else v)
    return hash(tuple(parts))


def send_material(mat, force=False):
    sig = material_signature(mat)
    if not force and STATE.material_sigs.get(mat.name) == sig:
        return
    STATE.material_sigs[mat.name] = sig
    header, payload = material_message(mat)
    STATE.conn.send(header, payload)


def send_mesh(ob, depsgraph):
    ob_eval = ob.evaluated_get(depsgraph)
    try:
        me = ob_eval.to_mesh()
    except RuntimeError:
        me = None
    header = {"type": "mesh", "object": ob.name, "primitives": []}
    chunks = []
    if me is not None:
        try:
            me.calc_loop_triangles()
            n_tris = len(me.loop_triangles)
            n_loops = len(me.loops)
            if n_tris:
                co = np.empty(len(me.vertices) * 3, np.float32)
                me.vertices.foreach_get("co", co)
                co = co.reshape(-1, 3)
                loop_vert = np.empty(n_loops, np.int32)
                me.loops.foreach_get("vertex_index", loop_vert)
                normals = np.empty(n_loops * 3, np.float32)
                me.corner_normals.foreach_get("vector", normals)
                normals = normals.reshape(-1, 3)
                uv = None
                if me.uv_layers.active is not None:
                    uv = np.empty(n_loops * 2, np.float32)
                    me.uv_layers.active.data.foreach_get("uv", uv)
                    uv = uv.reshape(-1, 2)
                    uv[:, 1] = 1.0 - uv[:, 1]
                tris = np.empty(n_tris * 3, np.int32)
                me.loop_triangles.foreach_get("loops", tris)
                tris = tris.reshape(-1, 3)
                mats = np.empty(n_tris, np.int32)
                me.loop_triangles.foreach_get("material_index", mats)

                # One vertex per corner, in the engine's frame.
                pos = co[loop_vert]
                pos = np.stack((pos[:, 0], pos[:, 2], -pos[:, 1]), axis=1)
                nrm = np.stack((normals[:, 0], normals[:, 2], -normals[:, 1]), axis=1)

                slots = [s.material.name if s.material else None for s in ob_eval.material_slots]
                for index in np.unique(mats):
                    corners = tris[mats == index].ravel()
                    used, remap = np.unique(corners, return_inverse=True)
                    chunks.append(np.ascontiguousarray(pos[used], "<f4").tobytes())
                    chunks.append(np.ascontiguousarray(nrm[used], "<f4").tobytes())
                    if uv is not None:
                        chunks.append(np.ascontiguousarray(uv[used], "<f4").tobytes())
                    chunks.append(np.ascontiguousarray(remap, "<u4").tobytes())
                    material = slots[index] if 0 <= index < len(slots) else None
                    if material is not None and material not in STATE.material_sigs:
                        send_material(bpy.data.materials[material])
                    header["primitives"].append({
                        "material": material,
                        "vertices": int(len(used)),
                        "indices": int(len(remap)),
                        "uv": uv is not None,
                    })
        finally:
            ob_eval.to_mesh_clear()
    STATE.conn.send(header, b"".join(chunks))
    STATE.mesh_sent_at[ob.name] = time.monotonic()


def object_record(ob, kind):
    record = {
        "ptr": ob.as_pointer(),
        "kind": kind,
        "matrix": ob.matrix_world.copy(),
        "visible": ob.visible_get(),
    }
    if kind == "light":
        record["light"] = light_header(ob)
    return record


def send_new(ob, depsgraph):
    kind = kind_of(ob)
    send_object(ob, kind)
    if kind == "light":
        STATE.conn.send(light_header(ob))
    elif kind == "mesh":
        send_mesh(ob, depsgraph)
    STATE.objects[ob.name] = object_record(ob, kind)
    STATE.names[ob.as_pointer()] = ob.name


def full_sync():
    STATE.reset()
    scene = bpy.context.scene
    depsgraph = bpy.context.evaluated_depsgraph_get()
    STATE.conn.send({
        "type": "hello",
        "file": bpy.path.basename(bpy.data.filepath) or "untitled.blend",
        "blender": bpy.app.version_string,
    })
    for ob in scene.objects:
        send_new(ob, depsgraph)
    STATE.conn.send({"type": "synced", "objects": [ob.name for ob in scene.objects]})


def warn_once(key, message):
    if key not in STATE.warned:
        STATE.warned.add(key)
        print("[Renzora Live Link] " + message)


# ---------------------------------------------------------------------------
# Watching for changes
# ---------------------------------------------------------------------------


@bpy.app.handlers.persistent
def on_depsgraph(scene, depsgraph):
    if not connected():
        return
    for update in depsgraph.updates:
        id_ = update.id
        if isinstance(id_, bpy.types.Object):
            if update.is_updated_geometry:
                STATE.dirty_meshes.add(id_.original.name)
        elif isinstance(id_, bpy.types.Material):
            STATE.dirty_materials.add(id_.original.name)
        elif isinstance(id_, bpy.types.ShaderNodeTree):
            tree = id_.original
            for mat in bpy.data.materials:
                if mat.node_tree == tree:
                    STATE.dirty_materials.add(mat.name)


@bpy.app.handlers.persistent
def on_load(_):
    # A different .blend opened while linked: link that one instead.
    if connected():
        full_sync()


def tick():
    if STATE.conn is None:
        return None
    if not STATE.conn.alive:
        STATE.status = "Disconnected: " + (STATE.conn.why or "link closed")
        STATE.conn = None
        redraw()
        return None
    try:
        step()
    except Exception as e:  # never let one bad object stop the link
        warn_once("tick:" + type(e).__name__, "error while syncing: %r" % e)
    return TICK


def step():
    scene = bpy.context.scene
    depsgraph = bpy.context.evaluated_depsgraph_get()
    current = {ob.name: ob for ob in scene.objects}

    # Renames: a pointer seen before under another name that is gone now.
    for name, ob in current.items():
        if name in STATE.objects:
            continue
        old = STATE.names.get(ob.as_pointer())
        if old and old != name and old not in current and old in STATE.objects:
            STATE.conn.send({"type": "rename", "from": old, "to": name})
            STATE.objects[name] = STATE.objects.pop(old)
            STATE.names[ob.as_pointer()] = name
            if old in STATE.mesh_sent_at:
                STATE.mesh_sent_at[name] = STATE.mesh_sent_at.pop(old)

    for name in [n for n in STATE.objects if n not in current]:
        STATE.conn.send({"type": "remove", "object": name})
        del STATE.objects[name]
        STATE.dirty_meshes.discard(name)

    for name, ob in current.items():
        record = STATE.objects.get(name)
        if record is None:
            send_new(ob, depsgraph)
            STATE.dirty_meshes.discard(name)
            continue
        kind = kind_of(ob)
        if kind != record["kind"]:
            send_new(ob, depsgraph)
            continue
        visible = ob.visible_get()
        if ob.matrix_world != record["matrix"] or visible != record["visible"]:
            send_object(ob, kind)
            record["matrix"] = ob.matrix_world.copy()
            record["visible"] = visible
        if kind == "light":
            header = light_header(ob)
            if header != record.get("light"):
                STATE.conn.send(header)
                record["light"] = header

    now = time.monotonic()
    for name in list(STATE.dirty_materials):
        mat = bpy.data.materials.get(name)
        STATE.dirty_materials.discard(name)
        if mat is not None and name in STATE.material_sigs:
            send_material(mat)
    for name in list(STATE.dirty_meshes):
        ob = current.get(name)
        if ob is None or kind_of(ob) != "mesh":
            STATE.dirty_meshes.discard(name)
            continue
        if now - STATE.mesh_sent_at.get(name, 0.0) >= MESH_EVERY:
            STATE.dirty_meshes.discard(name)
            send_mesh(ob, depsgraph)


def redraw():
    wm = getattr(bpy.context, "window_manager", None)
    if wm is None:
        return
    for window in wm.windows:
        for area in window.screen.areas:
            if area.type == "VIEW_3D":
                area.tag_redraw()


def connect(host, port):
    disconnect()
    try:
        STATE.conn = Connection(host, port)
    except OSError as e:
        STATE.status = "Could not connect: %s (is the editor running with the Blender Live Link plugin?)" % e
        return False
    STATE.status = "Connected"
    full_sync()
    if on_depsgraph not in bpy.app.handlers.depsgraph_update_post:
        bpy.app.handlers.depsgraph_update_post.append(on_depsgraph)
    if on_load not in bpy.app.handlers.load_post:
        bpy.app.handlers.load_post.append(on_load)
    if not bpy.app.timers.is_registered(tick):
        bpy.app.timers.register(tick, first_interval=TICK, persistent=True)
    return True


def disconnect():
    if STATE.conn is not None:
        STATE.conn.close()
        STATE.conn = None
        STATE.status = "Not connected"
    if bpy.app.timers.is_registered(tick):
        bpy.app.timers.unregister(tick)


# ---------------------------------------------------------------------------
# UI
# ---------------------------------------------------------------------------


class RENZORA_OT_connect(bpy.types.Operator):
    bl_idname = "renzora.live_link_connect"
    bl_label = "Connect"
    bl_description = "Link this scene to the running Renzora editor"

    def execute(self, context):
        wm = context.window_manager
        if connect(wm.renzora_host, wm.renzora_port):
            self.report({"INFO"}, "Linked to Renzora")
            return {"FINISHED"}
        self.report({"ERROR"}, STATE.status)
        return {"CANCELLED"}


class RENZORA_OT_disconnect(bpy.types.Operator):
    bl_idname = "renzora.live_link_disconnect"
    bl_label = "Disconnect"
    bl_description = "Stop sending changes; linked objects stay in the editor as saved models"

    def execute(self, context):
        disconnect()
        return {"FINISHED"}


class RENZORA_OT_resync(bpy.types.Operator):
    bl_idname = "renzora.live_link_resync"
    bl_label = "Resync"
    bl_description = "Send every object, mesh and material again"

    def execute(self, context):
        if connected():
            full_sync()
        return {"FINISHED"}


class RENZORA_PT_live_link(bpy.types.Panel):
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "Renzora"
    bl_label = "Live Link"

    def draw(self, context):
        layout = self.layout
        wm = context.window_manager
        if connected():
            project = STATE.conn.project
            layout.label(text="Linked to " + (project or "Renzora"), icon="LINKED")
            layout.label(text="%d objects" % len(STATE.objects))
            row = layout.row()
            row.operator(RENZORA_OT_resync.bl_idname, icon="FILE_REFRESH")
            row.operator(RENZORA_OT_disconnect.bl_idname, icon="UNLINKED")
        else:
            col = layout.column()
            col.prop(wm, "renzora_host", text="Host")
            col.prop(wm, "renzora_port", text="Port")
            layout.operator(RENZORA_OT_connect.bl_idname, icon="LINKED")
            layout.label(text=STATE.status)


CLASSES = (RENZORA_OT_connect, RENZORA_OT_disconnect, RENZORA_OT_resync, RENZORA_PT_live_link)

# ---------------------------------------------------------------------------
# Updating in place
# ---------------------------------------------------------------------------
#
# The editor rewrites this file whenever its plugin carries a newer version.
# Rather than wait for Blender to restart, the add-on notices its own file
# changed and has Blender reload it, reconnecting if it was linked. Blender's
# `addon_utils.enable` re-imports a module whose file is newer than the copy
# it loaded, which is what makes a disable/enable pair a reload.

_FILE = os.path.abspath(__file__)
try:
    _MTIME = os.path.getmtime(_FILE)
except OSError:
    _MTIME = 0.0
_RECONNECT = "renzora_live_link_reconnect"
WATCH_EVERY = 2.0


def watch_self():
    try:
        mtime = os.path.getmtime(_FILE)
    except OSError:
        return WATCH_EVERY
    if mtime == _MTIME:
        return WATCH_EVERY
    import addon_utils

    # Only an enabled add-on can be reloaded this way; loaded any other way,
    # the new version waits for the next start.
    if __name__ not in bpy.context.preferences.addons:
        return None
    if connected():
        wm = bpy.context.window_manager
        bpy.app.driver_namespace[_RECONNECT] = (wm.renzora_host, wm.renzora_port)
    print("[Renzora Live Link] a new version was installed; reloading")
    addon_utils.disable(__name__)
    addon_utils.enable(__name__)
    return None


def reconnect_after_reload():
    target = bpy.app.driver_namespace.pop(_RECONNECT, None)
    if target is not None:
        connect(*target)
    return None


def register():
    # Two copies can be loaded at once (one installed by hand under another
    # name, say). The first to load wins, and the second stays out of its way
    # rather than failing to register twice.
    if hasattr(bpy.types.WindowManager, "renzora_host"):
        return
    bpy.types.WindowManager.renzora_host = bpy.props.StringProperty(default="127.0.0.1")
    bpy.types.WindowManager.renzora_port = bpy.props.IntProperty(default=DEFAULT_PORT, min=1, max=65535)
    for cls in CLASSES:
        bpy.utils.register_class(cls)
    if not bpy.app.timers.is_registered(watch_self):
        bpy.app.timers.register(watch_self, first_interval=WATCH_EVERY, persistent=True)
    if _RECONNECT in bpy.app.driver_namespace:
        # Deferred: registration runs where connecting (which reads the scene)
        # is not allowed yet.
        bpy.app.timers.register(reconnect_after_reload, first_interval=0.2)


def unregister():
    if not getattr(RENZORA_PT_live_link, "is_registered", False):
        return
    if bpy.app.timers.is_registered(watch_self):
        bpy.app.timers.unregister(watch_self)
    disconnect()
    for handlers, fn in (
        (bpy.app.handlers.depsgraph_update_post, on_depsgraph),
        (bpy.app.handlers.load_post, on_load),
    ):
        if fn in handlers:
            handlers.remove(fn)
    for cls in reversed(CLASSES):
        bpy.utils.unregister_class(cls)
    del bpy.types.WindowManager.renzora_host
    del bpy.types.WindowManager.renzora_port


if __name__ == "__main__":
    register()
