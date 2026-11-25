// The value types scripts compute with: Vector2, Vector3, Color, Quaternion,
// and Mathf for the scalar helpers.
//
// Renzora's own rather than System.Numerics': these are what the engine's
// values decode to (a `Get("Transform.translation")` is a Vector3), they print
// the way the console shows vectors in every other language, and the names a
// game programmer expects (`Vector3.Up`, `Mathf.Lerp`) are there. A
// System.Numerics vector converts implicitly in both directions, for code that
// already uses one.

using System;
using System.Globalization;

namespace Renzora;

/// <summary>A 2D vector.</summary>
public readonly struct Vector2 : IEquatable<Vector2>
{
    public readonly float X, Y;

    public Vector2(float x, float y)
    {
        X = x;
        Y = y;
    }

    public static Vector2 Zero => new(0, 0);
    public static Vector2 One => new(1, 1);

    public float Length => MathF.Sqrt(X * X + Y * Y);
    public float LengthSquared => X * X + Y * Y;
    public Vector2 Normalized => Length > 0 ? this / Length : Zero;

    public static float Dot(Vector2 a, Vector2 b) => a.X * b.X + a.Y * b.Y;
    public static float Distance(Vector2 a, Vector2 b) => (a - b).Length;
    public static Vector2 Lerp(Vector2 a, Vector2 b, float t) => a + (b - a) * t;

    public static Vector2 operator +(Vector2 a, Vector2 b) => new(a.X + b.X, a.Y + b.Y);
    public static Vector2 operator -(Vector2 a, Vector2 b) => new(a.X - b.X, a.Y - b.Y);
    public static Vector2 operator -(Vector2 a) => new(-a.X, -a.Y);
    public static Vector2 operator *(Vector2 a, float s) => new(a.X * s, a.Y * s);
    public static Vector2 operator *(float s, Vector2 a) => new(a.X * s, a.Y * s);
    public static Vector2 operator /(Vector2 a, float s) => new(a.X / s, a.Y / s);
    public static bool operator ==(Vector2 a, Vector2 b) => a.Equals(b);
    public static bool operator !=(Vector2 a, Vector2 b) => !a.Equals(b);

    public static implicit operator System.Numerics.Vector2(Vector2 v) => new(v.X, v.Y);
    public static implicit operator Vector2(System.Numerics.Vector2 v) => new(v.X, v.Y);

    public void Deconstruct(out float x, out float y)
    {
        x = X;
        y = Y;
    }

    public bool Equals(Vector2 o) => X.Equals(o.X) && Y.Equals(o.Y);
    public override bool Equals(object? obj) => obj is Vector2 o && Equals(o);
    public override int GetHashCode() => HashCode.Combine(X, Y);
    public override string ToString() => string.Create(CultureInfo.InvariantCulture, $"Vector2({X}, {Y})");
}

/// <summary>A 3D vector. Y is up, and -Z is forward, as in the engine.</summary>
public readonly struct Vector3 : IEquatable<Vector3>
{
    public readonly float X, Y, Z;

    public Vector3(float x, float y, float z)
    {
        X = x;
        Y = y;
        Z = z;
    }

    public static Vector3 Zero => new(0, 0, 0);
    public static Vector3 One => new(1, 1, 1);
    public static Vector3 Up => new(0, 1, 0);
    public static Vector3 Down => new(0, -1, 0);
    public static Vector3 Right => new(1, 0, 0);
    public static Vector3 Left => new(-1, 0, 0);
    public static Vector3 Forward => new(0, 0, -1);
    public static Vector3 Back => new(0, 0, 1);

    public float Length => MathF.Sqrt(X * X + Y * Y + Z * Z);
    public float LengthSquared => X * X + Y * Y + Z * Z;
    public Vector3 Normalized => Length > 0 ? this / Length : Zero;

    public static float Dot(Vector3 a, Vector3 b) => a.X * b.X + a.Y * b.Y + a.Z * b.Z;
    public static Vector3 Cross(Vector3 a, Vector3 b) =>
        new(a.Y * b.Z - a.Z * b.Y, a.Z * b.X - a.X * b.Z, a.X * b.Y - a.Y * b.X);
    public static float Distance(Vector3 a, Vector3 b) => (a - b).Length;
    public static Vector3 Lerp(Vector3 a, Vector3 b, float t) => a + (b - a) * t;
    public static Vector3 Scale(Vector3 a, Vector3 b) => new(a.X * b.X, a.Y * b.Y, a.Z * b.Z);

    public static Vector3 operator +(Vector3 a, Vector3 b) => new(a.X + b.X, a.Y + b.Y, a.Z + b.Z);
    public static Vector3 operator -(Vector3 a, Vector3 b) => new(a.X - b.X, a.Y - b.Y, a.Z - b.Z);
    public static Vector3 operator -(Vector3 a) => new(-a.X, -a.Y, -a.Z);
    public static Vector3 operator *(Vector3 a, float s) => new(a.X * s, a.Y * s, a.Z * s);
    public static Vector3 operator *(float s, Vector3 a) => new(a.X * s, a.Y * s, a.Z * s);
    public static Vector3 operator /(Vector3 a, float s) => new(a.X / s, a.Y / s, a.Z / s);
    public static bool operator ==(Vector3 a, Vector3 b) => a.Equals(b);
    public static bool operator !=(Vector3 a, Vector3 b) => !a.Equals(b);

    public static implicit operator System.Numerics.Vector3(Vector3 v) => new(v.X, v.Y, v.Z);
    public static implicit operator Vector3(System.Numerics.Vector3 v) => new(v.X, v.Y, v.Z);

    public void Deconstruct(out float x, out float y, out float z)
    {
        x = X;
        y = Y;
        z = Z;
    }

    public bool Equals(Vector3 o) => X.Equals(o.X) && Y.Equals(o.Y) && Z.Equals(o.Z);
    public override bool Equals(object? obj) => obj is Vector3 o && Equals(o);
    public override int GetHashCode() => HashCode.Combine(X, Y, Z);
    public override string ToString() => string.Create(CultureInfo.InvariantCulture, $"Vector3({X}, {Y}, {Z})");
}

/// <summary>A linear RGBA colour, channels 0 to 1.</summary>
public readonly struct Color : IEquatable<Color>
{
    public readonly float R, G, B, A;

    public Color(float r, float g, float b, float a = 1f)
    {
        R = r;
        G = g;
        B = b;
        A = a;
    }

    public static Color White => new(1, 1, 1);
    public static Color Black => new(0, 0, 0);
    public static Color Red => new(1, 0, 0);
    public static Color Green => new(0, 1, 0);
    public static Color Blue => new(0, 0, 1);
    public static Color Clear => new(0, 0, 0, 0);

    /// <summary><c>#rrggbb</c> or <c>#rrggbbaa</c>; white when it cannot be read.</summary>
    public static Color Hex(string hex)
    {
        var s = hex.Trim().TrimStart('#');
        float Ch(int i) => s.Length >= i + 2 && byte.TryParse(s.AsSpan(i, 2), NumberStyles.HexNumber, null, out var b) ? b / 255f : 1f;
        return s.Length switch
        {
            6 => new Color(Ch(0), Ch(2), Ch(4)),
            8 => new Color(Ch(0), Ch(2), Ch(4), Ch(6)),
            _ => White,
        };
    }

    public static Color Lerp(Color a, Color b, float t) =>
        new(a.R + (b.R - a.R) * t, a.G + (b.G - a.G) * t, a.B + (b.B - a.B) * t, a.A + (b.A - a.A) * t);

    public static bool operator ==(Color a, Color b) => a.Equals(b);
    public static bool operator !=(Color a, Color b) => !a.Equals(b);

    public bool Equals(Color o) => R.Equals(o.R) && G.Equals(o.G) && B.Equals(o.B) && A.Equals(o.A);
    public override bool Equals(object? obj) => obj is Color o && Equals(o);
    public override int GetHashCode() => HashCode.Combine(R, G, B, A);
    public override string ToString() => string.Create(CultureInfo.InvariantCulture, $"Color({R}, {G}, {B}, {A})");
}

/// <summary>A rotation. <see cref="Script.Orientation"/> is the entity's.</summary>
public readonly struct Quaternion : IEquatable<Quaternion>
{
    public readonly float X, Y, Z, W;

    public Quaternion(float x, float y, float z, float w)
    {
        X = x;
        Y = y;
        Z = z;
        W = w;
    }

    public static Quaternion Identity => new(0, 0, 0, 1);

    /// <summary>Rotate a vector.</summary>
    public static Vector3 operator *(Quaternion q, Vector3 v)
    {
        var u = new Vector3(q.X, q.Y, q.Z);
        var t = 2f * Vector3.Cross(u, v);
        return v + q.W * t + Vector3.Cross(u, t);
    }

    public bool Equals(Quaternion o) => X.Equals(o.X) && Y.Equals(o.Y) && Z.Equals(o.Z) && W.Equals(o.W);
    public override bool Equals(object? obj) => obj is Quaternion o && Equals(o);
    public override int GetHashCode() => HashCode.Combine(X, Y, Z, W);
    public static bool operator ==(Quaternion a, Quaternion b) => a.Equals(b);
    public static bool operator !=(Quaternion a, Quaternion b) => !a.Equals(b);
    public override string ToString() => string.Create(CultureInfo.InvariantCulture, $"Quaternion({X}, {Y}, {Z}, {W})");
}

/// <summary>The scalar helpers every engine has, in float.</summary>
public static class Mathf
{
    public const float PI = MathF.PI;
    public const float Deg2Rad = MathF.PI / 180f;
    public const float Rad2Deg = 180f / MathF.PI;

    public static float Lerp(float a, float b, float t) => a + (b - a) * t;
    public static float Clamp(float v, float min, float max) => v < min ? min : v > max ? max : v;
    public static float Clamp01(float v) => Clamp(v, 0, 1);
    public static float MoveTowards(float current, float target, float maxDelta) =>
        MathF.Abs(target - current) <= maxDelta ? target : current + MathF.Sign(target - current) * maxDelta;
    public static float SmoothStep(float a, float b, float t)
    {
        t = Clamp01(t);
        t = t * t * (3f - 2f * t);
        return a + (b - a) * t;
    }
}
