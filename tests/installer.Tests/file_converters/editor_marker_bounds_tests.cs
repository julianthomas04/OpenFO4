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
        public void ARotationTurnsThePointsTheWayTheConverterReadsIt()
        {
            var scene = new NifSceneBuilder();
            scene.AddShape(scene.Root, "EditorMarker", [new Vector3(1, 0, 0), new Vector3(2, 0, 0), new Vector3(1, 0, 1)], OneTriangle, rotation: NifSceneBuilder.AboutZ(90));

            Assert.Equal(Box(new Vector3(0, 1, 0), new Vector3(0, 2, 1)), EditorMarkerBounds.Compute(scene.Nif));
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
