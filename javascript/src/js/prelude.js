// Runs once per interpreter, before any script.
//
// What lives here is what is easier to say in JavaScript than in Rust: the
// vector classes the API hands back, `console` routed into the engine console,
// how a value is shown when it is logged, and the canvas `on_draw(g)` receives.
// Natives come from the `renzora` object, which the plugin built before running
// this. Whatever this returns, the plugin keeps for its own conversions.
(function (rz) {
  "use strict";

  // A vector argument to a method: another vector, any {x, y, z} object, or an
  // array, so `a.add([1, 0, 0])` works as readily as `a.add(vec3(1, 0, 0))`.
  function px(o) { return Array.isArray(o) ? o[0] : o.x; }
  function py(o) { return Array.isArray(o) ? o[1] : o.y; }
  function pz(o) { return Array.isArray(o) ? (o[2] ?? 0) : (o.z ?? 0); }
  function fmt(n) { return Number.isInteger(n) ? String(n) : String(+n.toFixed(6)); }

  // JavaScript has no operator overloading, so `a + b` on two vectors is
  // string concatenation and nothing can change that. The arithmetic is
  // methods instead (`a.add(b)`, `v.mul(2)`), each returning a new vector and
  // leaving the receiver alone.
  class Vec2 {
    constructor(x = 0, y = 0) {
      if (typeof x === "object" && x !== null) { y = py(x); x = px(x); }
      this.x = +x;
      this.y = +y;
    }
    add(o) { return new Vec2(this.x + px(o), this.y + py(o)); }
    sub(o) { return new Vec2(this.x - px(o), this.y - py(o)); }
    mul(s) { return new Vec2(this.x * s, this.y * s); }
    div(s) { return new Vec2(this.x / s, this.y / s); }
    neg() { return new Vec2(-this.x, -this.y); }
    dot(o) { return this.x * px(o) + this.y * py(o); }
    length() { return Math.hypot(this.x, this.y); }
    lengthSquared() { return this.x * this.x + this.y * this.y; }
    normalized() { const n = this.length(); return n > 0 ? this.div(n) : new Vec2(); }
    distance(o) { return Math.hypot(this.x - px(o), this.y - py(o)); }
    lerp(o, t) { return new Vec2(this.x + (px(o) - this.x) * t, this.y + (py(o) - this.y) * t); }
    equals(o) { return this.x === px(o) && this.y === py(o); }
    clone() { return new Vec2(this.x, this.y); }
    toArray() { return [this.x, this.y]; }
    toString() { return `Vec2(${fmt(this.x)}, ${fmt(this.y)})`; }
    // Destructuring, as Lua's two return values: `const [x, y] = input_axis_2d("move")`.
    *[Symbol.iterator]() { yield this.x; yield this.y; }
  }

  class Vec3 {
    constructor(x = 0, y = 0, z = 0) {
      if (typeof x === "object" && x !== null) { z = pz(x); y = py(x); x = px(x); }
      this.x = +x;
      this.y = +y;
      this.z = +z;
    }
    add(o) { return new Vec3(this.x + px(o), this.y + py(o), this.z + pz(o)); }
    sub(o) { return new Vec3(this.x - px(o), this.y - py(o), this.z - pz(o)); }
    mul(s) { return new Vec3(this.x * s, this.y * s, this.z * s); }
    div(s) { return new Vec3(this.x / s, this.y / s, this.z / s); }
    neg() { return new Vec3(-this.x, -this.y, -this.z); }
    dot(o) { return this.x * px(o) + this.y * py(o) + this.z * pz(o); }
    cross(o) {
      const [x, y, z] = [px(o), py(o), pz(o)];
      return new Vec3(this.y * z - this.z * y, this.z * x - this.x * z, this.x * y - this.y * x);
    }
    length() { return Math.hypot(this.x, this.y, this.z); }
    lengthSquared() { return this.x * this.x + this.y * this.y + this.z * this.z; }
    normalized() { const n = this.length(); return n > 0 ? this.div(n) : new Vec3(); }
    distance(o) { return Math.hypot(this.x - px(o), this.y - py(o), this.z - pz(o)); }
    lerp(o, t) {
      return new Vec3(
        this.x + (px(o) - this.x) * t,
        this.y + (py(o) - this.y) * t,
        this.z + (pz(o) - this.z) * t,
      );
    }
    equals(o) { return this.x === px(o) && this.y === py(o) && this.z === pz(o); }
    clone() { return new Vec3(this.x, this.y, this.z); }
    toArray() { return [this.x, this.y, this.z]; }
    toString() { return `Vec3(${fmt(this.x)}, ${fmt(this.y)}, ${fmt(this.z)})`; }
    *[Symbol.iterator]() { yield this.x; yield this.y; yield this.z; }
  }

  // An RGBA colour, channels 0..1. Alpha defaults to opaque.
  class Color {
    constructor(r = 1, g = 1, b = 1, a = 1) {
      if (typeof r === "object" && r !== null) {
        const o = r;
        if (Array.isArray(o)) { [r, g, b, a] = [o[0], o[1], o[2], o[3] ?? 1]; }
        else { [r, g, b, a] = [o.r, o.g, o.b, o.a ?? 1]; }
      }
      this.r = +r;
      this.g = +g;
      this.b = +b;
      this.a = +a;
    }
    toArray() { return [this.r, this.g, this.b, this.a]; }
    toString() { return `Color(${fmt(this.r)}, ${fmt(this.g)}, ${fmt(this.b)}, ${fmt(this.a)})`; }
    *[Symbol.iterator]() { yield this.r; yield this.g; yield this.b; yield this.a; }
  }

  // -- How a value reads in the console ------------------------------------
  //
  // Close to Node's `util.inspect`, which is what a JavaScript programmer
  // expects `console.log({a: 1})` to print: `{ a: 1 }`, not
  // `[object Object]`. A top-level string prints bare; a nested one quoted.

  function inspect(value, depth = 0, seen = new Set()) {
    switch (typeof value) {
      case "string": return depth === 0 ? value : JSON.stringify(value);
      case "number": return Object.is(value, -0) ? "-0" : String(value);
      case "bigint": return `${value}n`;
      case "undefined": return "undefined";
      case "symbol": return value.toString();
      case "function": return value.name ? `[Function: ${value.name}]` : "[Function (anonymous)]";
      case "boolean": return String(value);
    }
    if (value === null) return "null";
    if (value instanceof Error) {
      // Boa's `stack` is the frames alone, where V8's starts with the
      // message, so the message is put back in front.
      const head = value.message ? `${value.name}: ${value.message}` : String(value.name);
      const stack = typeof value.stack === "string" ? value.stack.replace(/\s+$/, "") : "";
      if (!stack) return head;
      return stack.startsWith(head) ? stack : `${head}\n${stack}`;
    }
    if (value instanceof Vec2 || value instanceof Vec3 || value instanceof Color) return value.toString();
    if (seen.has(value)) return "[Circular]";
    if (depth > 3) return Array.isArray(value) ? "[Array]" : "[Object]";
    seen.add(value);
    try {
      if (Array.isArray(value)) {
        return `[${value.map((v) => inspect(v, depth + 1, seen)).join(", ")}]`;
      }
      if (value instanceof Map) {
        const items = [...value].map(([k, v]) => `${inspect(k, depth + 1, seen)} => ${inspect(v, depth + 1, seen)}`);
        return `Map(${value.size}) {${items.length ? " " + items.join(", ") + " " : ""}}`;
      }
      if (value instanceof Set) {
        const items = [...value].map((v) => inspect(v, depth + 1, seen));
        return `Set(${value.size}) {${items.length ? " " + items.join(", ") + " " : ""}}`;
      }
      if (value instanceof Date) return value.toISOString();
      // A class with its own toString says how it wants to be shown.
      const proto = Object.getPrototypeOf(value);
      if (proto !== null && proto !== Object.prototype && value.toString !== Object.prototype.toString) {
        return String(value);
      }
      const keys = Object.keys(value);
      if (keys.length === 0) return "{}";
      const body = keys.map((k) => {
        const name = /^[A-Za-z_$][\w$]*$/.test(k) ? k : JSON.stringify(k);
        return `${name}: ${inspect(value[k], depth + 1, seen)}`;
      });
      return `{ ${body.join(", ")} }`;
    } finally {
      seen.delete(value);
    }
  }

  // console.log("hp %d of %d", hp, max): the printf-style substitutions every
  // JavaScript console understands, then the rest of the arguments.
  function format(args) {
    if (args.length === 0) return "";
    let first = args[0];
    let rest = args.slice(1);
    let out;
    if (typeof first === "string" && first.includes("%")) {
      out = first.replace(/%[sdifoOjc%]/g, (spec) => {
        if (spec === "%%") return "%";
        if (rest.length === 0) return spec;
        const v = rest.shift();
        switch (spec) {
          case "%s": return typeof v === "string" ? v : inspect(v, 1);
          case "%d": case "%i": return String(Math.trunc(Number(v)));
          case "%f": return String(Number(v));
          case "%j": return JSON.stringify(v);
          case "%c": return "";
          default: return inspect(v, 1);
        }
      });
    } else {
      out = inspect(first);
    }
    for (const v of rest) out += " " + inspect(v);
    return out;
  }

  // -- console into the engine console --------------------------------------
  //
  // There is no terminal in the editor or a shipped game, and the plugin never
  // touches the process's standard streams: every line goes to the engine's
  // console as one log entry, at the level the method names.
  const counts = new Map();
  const timers = new Map();
  const console = {
    log: (...a) => rz._log("Info", format(a)),
    info: (...a) => rz._log("Info", format(a)),
    debug: (...a) => rz._log("Info", format(a)),
    warn: (...a) => rz._log("Warn", format(a)),
    error: (...a) => rz._log("Error", format(a)),
    trace: (...a) => rz._log("Info", format(a) + "\n" + (new Error().stack ?? "")),
    assert: (cond, ...a) => {
      if (!cond) rz._log("Error", "Assertion failed" + (a.length ? ": " + format(a) : ""));
    },
    count: (label = "default") => {
      const n = (counts.get(label) ?? 0) + 1;
      counts.set(label, n);
      rz._log("Info", `${label}: ${n}`);
    },
    countReset: (label = "default") => counts.delete(label),
    time: (label = "default") => timers.set(label, Date.now()),
    timeEnd: (label = "default") => {
      const t = timers.get(label);
      if (t !== undefined) {
        timers.delete(label);
        rz._log("Info", `${label}: ${Date.now() - t}ms`);
      }
    },
  };

  // -- on_draw(g) -------------------------------------------------------------

  // The `g` that on_draw(g) receives. Every method draws one shape this frame.
  class Canvas {}
  Canvas.prototype.line = rz._draw_line;
  Canvas.prototype.arc = rz._draw_arc;
  Canvas.prototype.circle = rz._draw_circle;
  Canvas.prototype.rect = rz._draw_rect;
  Canvas.prototype.text = rz._draw_text;
  Canvas.prototype.triangle = rz._draw_triangle;
  Canvas.prototype.poly = rz._draw_poly;

  const vec2 = (x, y) => new Vec2(x, y);
  const vec3 = (x, y, z) => new Vec3(x, y, z);
  const color = (r, g, b, a) => new Color(r, g, b, a);
  for (const [name, value] of Object.entries({ Vec2, Vec3, Color, vec2, vec3, color })) {
    rz[name] = value;
  }
  globalThis.console = console;

  return { vec2: Vec2.prototype, vec3: Vec3.prototype, color: Color.prototype, canvas: Canvas.prototype, inspect };
})
