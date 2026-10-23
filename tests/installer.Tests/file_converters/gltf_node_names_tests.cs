using System.Text.Json.Nodes;
using AssetConverter;
using SharpGLTF.Schema2;
using Xunit;

namespace InstallerTests
{
    // The engine addresses animated/skinned asset nodes by name, so duplicates must be
    // disambiguated deterministically; see the GltfNodeNames header comment.
    public class GltfNodeNamesTests
    {
        [Fact]
        public void EnsureUnique_ModelWithoutAnimationOrSkin_LeavesDuplicatesAlone()
        {
            var model = ModelRoot.CreateModel();
            var scene = model.UseScene("s");
            scene.CreateNode("dup");
            scene.CreateNode("dup");

            GltfNodeNames.EnsureUnique(model);

            Assert.Equal(new[] { "dup", "dup" }, NamesInIndexOrder(model));
        }

        [Fact]
        public void EnsureUnique_AnimatedModelWithDuplicates_KeepsFirstAndSuffixesTheRestInIndexOrder()
        {
            var model = ModelWithAnimation();
            var scene = model.UseScene("s");
            scene.CreateNode("dup");
            scene.CreateNode("dup");
            scene.CreateNode("dup");

            GltfNodeNames.EnsureUnique(model);

            Assert.Equal(new[] { "dup", "dup~2", "dup~3" }, NamesInIndexOrder(model));
        }

        [Fact]
        public void EnsureUnique_NodeWithExtrasAmongDuplicates_KeepsTheOriginalNameOnThatNode()
        {
            var model = ModelWithAnimation();
            var scene = model.UseScene("s");
            scene.CreateNode("dup");
            scene.CreateNode("dup");
            var animated = scene.CreateNode("dup");
            animated.Extras = JsonNode.Parse("{\"animated\":true}");

            GltfNodeNames.EnsureUnique(model);

            Assert.Equal(new[] { "dup~2", "dup~3", "dup" }, NamesInIndexOrder(model));
        }

        [Fact]
        public void EnsureUnique_MeshFreeDuplicateAmongMeshNodes_KeepsTheNameOnTheMeshFreeNode()
        {
            var model = ModelWithAnimation();
            var scene = model.UseScene("s");
            var mesh = model.CreateMesh("m");
            scene.CreateNode("dup").Mesh = mesh;
            scene.CreateNode("dup");

            GltfNodeNames.EnsureUnique(model);

            Assert.Equal(new[] { "dup~2", "dup" }, NamesInIndexOrder(model));
        }

        [Fact]
        public void EnsureUnique_UniqueAndEmptyNames_AreNotTouched()
        {
            var model = ModelWithAnimation();
            var scene = model.UseScene("s");
            scene.CreateNode("a");
            scene.CreateNode("b");
            scene.CreateNode("");
            scene.CreateNode("");

            GltfNodeNames.EnsureUnique(model);

            Assert.Equal(new[] { "a", "b", "", "" }, NamesInIndexOrder(model));
        }

        [Fact]
        public void EnsureUnique_TwoIndependentDuplicateGroups_AreResolvedSeparately()
        {
            var model = ModelWithAnimation();
            var scene = model.UseScene("s");
            scene.CreateNode("x");
            scene.CreateNode("y");
            scene.CreateNode("x");
            scene.CreateNode("y");

            GltfNodeNames.EnsureUnique(model);

            Assert.Equal(new[] { "x", "y", "x~2", "y~2" }, NamesInIndexOrder(model));
        }

        private static ModelRoot ModelWithAnimation()
        {
            var model = ModelRoot.CreateModel();
            model.CreateAnimation("clip");
            return model;
        }

        private static string[] NamesInIndexOrder(ModelRoot model) =>
            model.LogicalNodes.OrderBy(n => n.LogicalIndex).Select(n => n.Name ?? "").ToArray();
    }
}
