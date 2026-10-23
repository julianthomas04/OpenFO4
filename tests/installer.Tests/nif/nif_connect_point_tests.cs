using System.Numerics;
using System.Text.Json.Nodes;
using AssetConverter;
using InstallerTests.Archives;
using SharpGLTF.Schema2;
using Xunit;

namespace InstallerTests.Nif
{
    // Attach data of weapon parts as node extras.
    [Collection(StaticStateCollections.NifConverterState)]
    public class NifConnectPointTests
    {
        const float Tolerance = 1e-6f;

        // The baseball bat's p-Material point: 10 degrees about the NIF Y axis, stored as w 0.9961947, y -0.0871557.
        static readonly Quaternion BatMaterialRotation = new(0f, -0.0871557f, 0f, 0.9961947f);

        static ModelRoot Convert(ConnectPointNif nif)
        {
            using var dir = new ArchiveTempDir();
            string source = Path.Combine(dir.Path, "model.nif");
            File.WriteAllBytes(source, nif.ToBytes());
            string destination = Path.Combine(dir.Path, "model.glb");
            NifConverter.ConvertToGltf(source, destination);
            return ModelRoot.Load(destination);
        }

        static JsonObject? NodeExtras(ModelRoot model, string node) =>
            ExtrasOf(model.LogicalNodes.Single(n => n.Name == node));

        static JsonObject? ExtrasOf(Node node) => node.Extras == null ? null : JsonNode.Parse(node.Extras.ToJsonString()) as JsonObject;

        static JsonObject Extras(ModelRoot model, string node, string key) => (JsonObject)NodeExtras(model, node)![key]!;

        static float[] Floats(JsonNode? array) => array!.AsArray().Select(v => v!.GetValue<float>()).ToArray();

        static string[] Names(JsonArray points) => points.Select(p => p!["name"]!.GetValue<string>()).ToArray();

        static void AssertFloats(float[] expected, float[] actual)
        {
            Assert.Equal(expected.Length, actual.Length);
            for (int i = 0; i < expected.Length; i++)
                Assert.Equal(expected[i], actual[i], Tolerance);
        }

        [Fact]
        public void ConnectParents_WithoutParentName_LandOnTheRootInGltfSpaceAndEngineUnits()
        {
            var nif = new ConnectPointNif();
            nif.Parents.Add(new("", "p-Material", BatMaterialRotation, new Vector3(10, 20, 30)));

            var extras = Extras(Convert(nif), "Root", NifNodeExtras.ConnectParentsKey);

            Assert.Equal(1, extras["v"]!.GetValue<int>());
            var point = extras["points"]!.AsArray().Single()!;
            Assert.Equal("p-Material", point["name"]!.GetValue<string>());
            AssertFloats([1f, 3f, -2f], Floats(point["translation"]));
            AssertFloats([0f, 0f, 0.0871557f, 0.9961947f], Floats(point["rotation"]));
            Assert.Equal(1f, point["scale"]!.GetValue<float>());
        }

        [Fact]
        public void ConnectParents_NamingAChildNode_LandOnThatNodeAndKeepBlockOrderAndDuplicates()
        {
            var nif = new ConnectPointNif();
            nif.Parents.Add(new("Receiver", "P-Scope", Quaternion.Identity, Vector3.Zero));
            nif.Parents.Add(new("", "P-Receiver", Quaternion.Identity, Vector3.Zero));
            nif.Parents.Add(new("Receiver", "P-Scope", Quaternion.Identity, new Vector3(0, 10, 0)));
            var model = Convert(nif);

            var onReceiver = Extras(model, "Receiver", NifNodeExtras.ConnectParentsKey)["points"]!.AsArray();
            var onRoot = Extras(model, "Root", NifNodeExtras.ConnectParentsKey)["points"]!.AsArray();

            Assert.Equal(["P-Scope", "P-Scope"], Names(onReceiver));
            AssertFloats([0f, 0f, 0f], Floats(onReceiver[0]!["translation"]));
            AssertFloats([0f, 0f, -1f], Floats(onReceiver[1]!["translation"]));
            Assert.Equal(["P-Receiver"], Names(onRoot));
        }

        [Fact]
        public void ConnectParents_NamingAMissingNode_AreDroppedAndCounted()
        {
            NifAnimationStats.Reset();
            var nif = new ConnectPointNif();
            nif.Parents.Add(new("NoSuchNode", "P-Grip", Quaternion.Identity, Vector3.Zero));
            nif.Parents.Add(new("", "P-Mag", Quaternion.Identity, Vector3.Zero));

            var model = Convert(nif);

            Assert.Equal(["P-Mag"], Names(Extras(model, "Root", NifNodeExtras.ConnectParentsKey)["points"]!.AsArray()));
            Assert.Null(NodeExtras(model, "Receiver")?[NifNodeExtras.ConnectParentsKey]);
            Assert.Contains($"{NifNodeExtras.UnresolvedConnectParentCounter}: 1", NifAnimationStats.SummaryLines());
        }

        [Fact]
        public void ConnectChildren_CarryNamesSkinnedFlagAndTheRootTransformTheConverterStrips()
        {
            var nif = new ConnectPointNif { ChildNames = ["C-Grip", "C-Reciever"], Skinned = true, RootTranslation = new Vector3(10, 20, 30), RootScale = 2f };

            var model = Convert(nif);
            var extras = Extras(model, "Root", NifNodeExtras.ConnectChildrenKey);

            Assert.Equal(["C-Grip", "C-Reciever"], extras["names"]!.AsArray().Select(n => n!.GetValue<string>()));
            Assert.True(extras["skinned"]!.GetValue<bool>());
            var root = extras["root"]!;
            AssertFloats([1f, 3f, -2f], Floats(root["translation"]));
            AssertFloats([0f, 0f, 0f, 1f], Floats(root["rotation"]));
            Assert.Equal(2f, root["scale"]!.GetValue<float>(), Tolerance);
            Assert.Equal(Vector3.Zero, model.LogicalNodes.Single(n => n.Name == "Root").LocalTransform.Translation);
        }

        [Fact]
        public void ConnectParents_NamingABillboardNode_LandOnThatNode()
        {
            var nif = new ConnectPointNif { ReceiverIsBillboard = true };
            nif.Parents.Add(new("Receiver", "P-Grip", Quaternion.Identity, Vector3.Zero));

            var model = Convert(nif);

            Assert.Equal(["P-Grip"], Names(Extras(model, "Receiver", NifNodeExtras.ConnectParentsKey)["points"]!.AsArray()));
        }

        [Fact]
        public void ConnectPoints_MergeIntoTheExistingExtrasOfTheRoot()
        {
            var nif = new ConnectPointNif { RootHidden = true, ChildNames = ["C-Grip"] };
            nif.Parents.Add(new("", "P-Mag", Quaternion.Identity, Vector3.Zero));

            var rootExtras = NodeExtras(Convert(nif), "Root")!;

            Assert.True(rootExtras[NifNodeExtras.HiddenKey]!.GetValue<bool>());
            Assert.Equal(["P-Mag"], Names(rootExtras[NifNodeExtras.ConnectParentsKey]!["points"]!.AsArray()));
            Assert.Equal(["C-Grip"], rootExtras[NifNodeExtras.ConnectChildrenKey]!["names"]!.AsArray().Select(n => n!.GetValue<string>()));
        }

        [Fact]
        public void UnreadableConnectPointBlock_IsDroppedAndCountedWhileTheOtherBlockStillConverts()
        {
            NifAnimationStats.Reset();
            var nif = new ConnectPointNif { TruncateParentsPayload = true, ChildNames = ["C-Grip"] };
            nif.Parents.Add(new("", "P-Mag", Quaternion.Identity, Vector3.Zero));

            var model = Convert(nif);

            Assert.Null(NodeExtras(model, "Root")![NifNodeExtras.ConnectParentsKey]);
            Assert.NotNull(NodeExtras(model, "Root")![NifNodeExtras.ConnectChildrenKey]);
            Assert.Contains($"{NifNodeExtras.UnreadableConnectPointBlockCounter}: 1", NifAnimationStats.SummaryLines());
        }

        [Fact]
        public void NifWithoutConnectPoints_HasNoConnectExtras()
        {
            var model = Convert(new ConnectPointNif());

            Assert.All(model.LogicalNodes, n => Assert.Null(ExtrasOf(n)));
        }
    }
}
