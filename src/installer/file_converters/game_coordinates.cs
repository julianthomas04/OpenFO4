using System.Numerics;
using NiflySharp.Structs;

namespace AssetConverter
{
    /// <summary>
    /// Single source of truth for the conversion from Gamebryo/Creation Engine space (Z-up, game units)
    /// to the converted glTF space (Y-up, 0.1 units): position (x, y, z) -> (x, z, -y) * Scaling;
    /// rotation (x, y, z, w) -> (x, z, -y, w). Node transforms, skin bind matrices, animation keys,
    /// Havok data and collision all go through here so they agree.
    /// </summary>
    public static class GameCoordinates
    {
        public const float Scaling = 0.1f;

        /// <summary>1 Havok unit = 7 game units.</summary>
        public const float HavokUnitsInGameUnits = 7.0f;

        public static float HavokToGltfLength(float havok) => havok * HavokUnitsInGameUnits * Scaling;

        public static Vector3 ToGltfPosition(Vector3 v) => new(v.X * Scaling, v.Z * Scaling, -v.Y * Scaling);

        /// <summary>Position given in Havok units (axes as in game space).</summary>
        public static Vector3 HavokToGltfPosition(Vector3 v)
            => new(HavokToGltfLength(v.X), HavokToGltfLength(v.Z), -HavokToGltfLength(v.Y));

        /// <summary>Axis permutation of a unit-less direction (normals): no unit scaling.</summary>
        public static Vector3 ToGltfDirection(Vector3 v) => new(v.X, v.Z, -v.Y);

        public static Quaternion ToGltfRotation(Quaternion q) => new(q.X, q.Z, -q.Y, q.W);

        /// <summary>Scale is a per-axis factor, so it only follows the axis permutation (no sign flip, no unit scaling).</summary>
        public static Vector3 ToGltfScale(Vector3 s) => new(s.X, s.Z, s.Y);

        public static Matrix4x4 ToGltfTransform(Matrix33 rotation, Vector3 translation, float scale)
        {
            var matR = new Matrix4x4(
                rotation.M11, rotation.M13, -rotation.M12, 0,
                rotation.M31, rotation.M33, -rotation.M32, 0,
                -rotation.M21, -rotation.M23, rotation.M22, 0,
                0, 0, 0, 1);
            return Matrix4x4.CreateScale(scale) * matR * Matrix4x4.CreateTranslation(ToGltfPosition(translation));
        }

        // NiflySharp reads NIF quaternions (stored w, x, y, z) into System.Numerics
        // fields in file order, so an identity arrives as (X=1, Y=0, Z=0, W=0).
        public static Quaternion FromNiflySharp(Quaternion stored)
            => new(stored.Y, stored.Z, stored.W, stored.X);
    }
}
