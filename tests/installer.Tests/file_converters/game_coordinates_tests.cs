using System.Numerics;
using AssetConverter;
using NiflySharp.Structs;
using Xunit;

namespace InstallerTests
{
    // Convention: GameCoordinates class summary. Game space is Z-up, glTF is Y-up,
    // (x, y, z) -> (x, z, -y); positions are additionally scaled by 0.1.
    public class GameCoordinatesTests
    {
        private const float Tolerance = 1e-5f;

        [Fact]
        public void ToGltfPosition_GameUnits_SwapsYZNegatesZAndScalesByTenth()
        {
            var gltf = GameCoordinates.ToGltfPosition(new Vector3(10f, 20f, 30f));

            AssertVector(new Vector3(1f, 3f, -2f), gltf);
        }

        [Fact]
        public void ToGltfDirection_Normal_PermutesAxesWithoutScaling()
        {
            var gltf = GameCoordinates.ToGltfDirection(new Vector3(1f, 2f, 3f));

            AssertVector(new Vector3(1f, 3f, -2f), gltf);
        }

        [Fact]
        public void ToGltfScale_PerAxisFactor_SwapsYZWithoutSignFlip()
        {
            var gltf = GameCoordinates.ToGltfScale(new Vector3(1f, 2f, 3f));

            AssertVector(new Vector3(1f, 3f, 2f), gltf);
        }

        [Fact]
        public void HavokToGltfPosition_HavokUnits_ScalesBySevenGameUnitsThenToGltf()
        {
            var gltf = GameCoordinates.HavokToGltfPosition(new Vector3(1f, 2f, 3f));

            AssertVector(new Vector3(0.7f, 2.1f, -1.4f), gltf);
        }

        [Fact]
        public void ToGltfRotation_Quaternion_PermutesAxisComponentsAndKeepsW()
        {
            var gltf = GameCoordinates.ToGltfRotation(new Quaternion(1f, 2f, 3f, 4f));

            Assert.Equal(new Quaternion(1f, 3f, -2f, 4f), gltf);
        }

        [Fact]
        public void FromNiflySharp_FileOrderQuaternion_ReordersWxyzToXyzw()
        {
            // NIF stores w, x, y, z; NiflySharp fills X=w, Y=x, Z=y, W=z.
            var converted = GameCoordinates.FromNiflySharp(new Quaternion(1f, 2f, 3f, 4f));

            Assert.Equal(new Quaternion(2f, 3f, 4f, 1f), converted);
        }

        [Fact]
        public void FromNiflySharp_StoredIdentity_YieldsIdentityRotation()
        {
            var converted = GameCoordinates.FromNiflySharp(new Quaternion(1f, 0f, 0f, 0f));

            Assert.Equal(Quaternion.Identity, converted);
        }

        // Invariant: converting a rotation then rotating a converted vector equals
        // rotating in game space then converting the result (the change of basis commutes).
        [Theory]
        [InlineData(1f, 0f, 0f, 0.7f)]
        [InlineData(0f, 1f, 0f, 1.9f)]
        [InlineData(0f, 0f, 1f, -2.4f)]
        [InlineData(1f, 2f, 3f, 0.3f)]
        public void ToGltfRotation_ThenRotate_EqualsRotateThenConvert(float ax, float ay, float az, float angle)
        {
            var q = Quaternion.CreateFromAxisAngle(Vector3.Normalize(new Vector3(ax, ay, az)), angle);
            var v = new Vector3(0.3f, -1.1f, 2.5f);

            var convertedThenRotated = Vector3.Transform(GameCoordinates.ToGltfDirection(v), GameCoordinates.ToGltfRotation(q));
            var rotatedThenConverted = GameCoordinates.ToGltfDirection(Vector3.Transform(v, q));

            AssertVector(rotatedThenConverted, convertedThenRotated);
        }

        // NIF node transform: world = R * (scale * p) + t with column vectors.
        [Fact]
        public void ToGltfTransform_NodeTransform_MapsLocalGltfPointsToConvertedParentPoints()
        {
            // 90 degrees about game Z, (x, y, z) -> (-y, x, z), i.e. R = [[0,-1,0],[1,0,0],[0,0,1]].
            // NiflySharp names fields in nif.xml file order (M21 is the second value of the first row),
            // so the standard element R[0][1] is M21 and R[1][0] is M12.
            var rotation = new Matrix33
            {
                M11 = 0f,
                M21 = -1f,
                M31 = 0f,
                M12 = 1f,
                M22 = 0f,
                M32 = 0f,
                M13 = 0f,
                M23 = 0f,
                M33 = 1f,
            };
            var translation = new Vector3(10f, 20f, 30f);
            const float scale = 2f;
            var localGamePoint = new Vector3(1f, 2f, 3f);

            var transform = GameCoordinates.ToGltfTransform(rotation, translation, scale);

            // R * (2 * (1, 2, 3)) = R * (2, 4, 6) = (-4, 2, 6)
            var parentGamePoint = new Vector3(-4f, 2f, 6f) + translation;
            var actual = Vector3.Transform(GameCoordinates.ToGltfPosition(localGamePoint), transform);
            AssertVector(GameCoordinates.ToGltfPosition(parentGamePoint), actual);
        }

        [Fact]
        public void ToGltfTransform_IdentityRotationUnitScale_IsPureConvertedTranslation()
        {
            var identity = new Matrix33
            {
                M11 = 1f,
                M22 = 1f,
                M33 = 1f,
            };

            var transform = GameCoordinates.ToGltfTransform(identity, new Vector3(10f, 20f, 30f), 1f);

            Assert.Equal(Matrix4x4.CreateTranslation(1f, 3f, -2f), transform);
        }

        private static void AssertVector(Vector3 expected, Vector3 actual)
        {
            Assert.Equal(expected.X, actual.X, Tolerance);
            Assert.Equal(expected.Y, actual.Y, Tolerance);
            Assert.Equal(expected.Z, actual.Z, Tolerance);
        }
    }
}
