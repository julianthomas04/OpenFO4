using System.Numerics;
using AssetConverter;
using InstallerTests.Nif;
using NiflySharp.Structs;
using Xunit;

namespace InstallerTests
{
    public sealed class EditorMarkerBoundsTests
    {
        static readonly Triangle[] OneTriangle = [new Triangle(0, 1, 2)];

        static Vector3[] Corners(Vector3 min, Vector3 max) => [min, max, new Vector3(min.X, max.Y, min.Z)];

        static ObjectBoundsJson.FloatBox Box(Vector3 min, Vector3 max) =>
            new(new ObjectBoundsJson.FloatPoint(min.X, min.Y, min.Z), new ObjectBoundsJson.FloatPoint(max.X, max.Y, max.Z));

        [Fact]
        public void TheBoxAroundTheMarkerShapeIsReturned()
        {
            var scene = new NifSceneBuilder();
            scene.AddShape(scene.Root, "EditorMarker", Corners(new Vector3(-64, -8, 0), new Vector3(64, 8, 192)), OneTriangle);

            Assert.Equal(Box(new Vector3(-64, -8, 0), new Vector3(64, 8, 192)), EditorMarkerBounds.Compute(scene.Nif));
        }

        [Fact]
        public void OnlyShapesNamedEditorMarkerCount_AnywhereInTheName_AnyCase()
        {
            var scene = new NifSceneBuilder();
            scene.AddShape(scene.Root, "Door", Corners(new Vector3(-500, -500, -500), new Vector3(500, 500, 500)), OneTriangle);
            scene.AddShape(scene.Root, "VisibilityEditorMarker:0", Corners(new Vector3(0, 0, 0), new Vector3(1, 2, 3)), OneTriangle);
            scene.AddShape(scene.Root, "editormarker_b", Corners(new Vector3(-1, 0, 0), new Vector3(0, 5, 1)), OneTriangle);

            Assert.Equal(Box(new Vector3(-1, 0, 0), new Vector3(1, 5, 3)), EditorMarkerBounds.Compute(scene.Nif));
        }

        [Fact]
        public void AModelWithoutMarkerGeometryHasNoBounds()
        {
            var scene = new NifSceneBuilder();
            scene.AddShape(scene.Root, "Door", Corners(Vector3.Zero, Vector3.One), OneTriangle);

            Assert.Null(EditorMarkerBounds.Compute(scene.Nif));
        }

        [Fact]
        public void TheShapeAndItsAncestorsTransformsApplyButTheRootsDoesNot()
        {
            var scene = new NifSceneBuilder();
            var group = scene.AddNode(scene.Root, "Group", new Vector3(10, 0, 0), scale: 2f);
            scene.AddShape(group, "EditorMarker", Corners(Vector3.Zero, new Vector3(4, 2, 1)), OneTriangle, translation: new Vector3(0, 0, 100));

            Assert.Equal(Box(new Vector3(10, 0, 200), new Vector3(18, 4, 202)), EditorMarkerBounds.Compute(scene.Nif));
        }

        [Fact]
        public void AMarkerShapeOutsideAnyNodeKeepsItsVerticesAsTheConverterDoes()
        {
            var scene = new NifSceneBuilder();
            scene.AddRootShape("EditorMarker", Corners(new Vector3(-64, -8, 0), new Vector3(64, 8, 192)), OneTriangle,
                translation: new Vector3(500, 0, 0), rotation: NifSceneBuilder.AboutZ(90), scale: 3f);

            Assert.Equal(Box(new Vector3(-64, -8, 0), new Vector3(64, 8, 192)), EditorMarkerBounds.Compute(scene.Nif));
        }

        [Fact]
        public void ATransformPlacesThePointsWhereTheConverterPutsThem()
        {
            var rotation = Rotation(Matrix4x4.CreateFromYawPitchRoll(0.3f, 0.5f, 0.7f));
            var translation = new Vector3(10, -20, 30);
            const float Scale = 1.5f;
            Vector3[] points = [new(1, 0, 0), new(2, 3, -1), new(-4, 1, 5)];
            var scene = new NifSceneBuilder();
            scene.AddShape(scene.Root, "EditorMarker", points, OneTriangle, translation: translation, rotation: rotation, scale: Scale);

            var converted = GameCoordinates.ToGltfTransform(rotation, translation, Scale);
            var expected = points.Select(point => FromGltfPosition(Vector3.Transform(GameCoordinates.ToGltfPosition(point), converted))).ToArray();
            var bounds = EditorMarkerBounds.Compute(scene.Nif)!.Value;

            AssertNear(expected.Aggregate(Vector3.Min), bounds.Min);
            AssertNear(expected.Aggregate(Vector3.Max), bounds.Max);
        }

        static Matrix33 Rotation(Matrix4x4 m) => new()
        {
            M11 = m.M11,
            M12 = m.M12,
            M13 = m.M13,
            M21 = m.M21,
            M22 = m.M22,
            M23 = m.M23,
            M31 = m.M31,
            M32 = m.M32,
            M33 = m.M33,
        };

        static Vector3 FromGltfPosition(Vector3 gltf) => new Vector3(gltf.X, -gltf.Z, gltf.Y) / GameCoordinates.Scaling;

        static void AssertNear(Vector3 expected, ObjectBoundsJson.FloatPoint actual)
        {
            Assert.Equal(expected.X, actual.X, 0.002f);
            Assert.Equal(expected.Y, actual.Y, 0.002f);
            Assert.Equal(expected.Z, actual.Z, 0.002f);
        }

        [Fact]
        public void FractionalExtentsAreKeptToThreeDecimals()
        {
            var scene = new NifSceneBuilder();
            scene.AddShape(scene.Root, "EditorMarker", Corners(new Vector3(-0.12345f, 0, 0), new Vector3(0.5f, 1, 1)), OneTriangle);

            Assert.Equal(-0.123f, EditorMarkerBounds.Compute(scene.Nif)!.Value.Min.X);
        }

        [Fact]
        public void BytesThatAreNotANifAreRefused()
        {
            Assert.Throws<InvalidDataException>(() => EditorMarkerBounds.Compute([1, 2, 3, 4]));
        }

        [Fact]
        public void ASavedNifGivesTheSameBoundsThroughItsBytes()
        {
            using var directory = new TempDirectory();
            var scene = new NifSceneBuilder();
            scene.AddShape(scene.Root, "EditorMarker", Corners(new Vector3(-64, -8, 0), new Vector3(64, 8, 192)), OneTriangle);

            var bounds = EditorMarkerBounds.Compute(File.ReadAllBytes(scene.Save(directory.Path, "marker.nif")));

            Assert.Equal(Box(new Vector3(-64, -8, 0), new Vector3(64, 8, 192)), bounds);
        }
    }
}
