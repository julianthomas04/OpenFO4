using System.Numerics;
using System.Text.Json.Nodes;
using AssetConverter;
using SharpGLTF.Schema2;
using Xunit;

namespace InstallerTests.Nif
{
    [Collection(StaticStateCollections.NifConverterState)]
    public class GltfAnimationWriterExtrasTests
    {
        static SequenceClip ClipDriving(string node) => new(
            "Idle", 0f, 1f, "loop", [],
            [new NodeTrack(node, [new Sample<Vector3>(0f, Vector3.Zero), new Sample<Vector3>(1f, Vector3.UnitX)], [], [])]);

        static ModelRoot ModelWithNode(string name, JsonObject? extras)
        {
            var model = ModelRoot.CreateModel();
            var node = model.UseScene("scene").CreateNode(name);
            node.Extras = extras;
            return model;
        }

        [Fact]
        public void AnimatedNode_KeepsExtrasWrittenEarlierAndGainsAnim()
        {
            var connect = new JsonObject { ["v"] = 1, ["points"] = new JsonArray(new JsonObject { ["name"] = "P-Melee" }) };
            var model = ModelWithNode("Bone03", new JsonObject { [NifNodeExtras.ConnectParentsKey] = connect, [NifNodeExtras.HiddenKey] = true });

            GltfAnimationWriter.Write(model, [ClipDriving("Bone03")], _ => { });

            var extras = (JsonObject)model.LogicalNodes.Single().Extras!;
            Assert.True(extras["anim"]!.GetValue<bool>());
            Assert.True(extras[NifNodeExtras.HiddenKey]!.GetValue<bool>());
            Assert.Equal("P-Melee", extras[NifNodeExtras.ConnectParentsKey]!["points"]![0]!["name"]!.GetValue<string>());
        }

        [Fact]
        public void AnimatedNodeWithoutExtras_GetsAnimOnly()
        {
            var model = ModelWithNode("Bone03", null);

            GltfAnimationWriter.Write(model, [ClipDriving("Bone03")], _ => { });

            var extras = (JsonObject)model.LogicalNodes.Single().Extras!;
            Assert.Equal(["anim"], extras.Select(p => p.Key));
        }

        [Fact]
        public void NodeExtrasThatAreNotAnObject_AreRejectedInsteadOfReplaced()
        {
            Assert.Throws<InvalidOperationException>(() => GltfNodeExtras.Existing(new JsonArray()));
        }
    }
}
