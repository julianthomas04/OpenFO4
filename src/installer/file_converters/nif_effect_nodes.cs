using System.Text.Json.Nodes;
using NiflySharp;
using NiflySharp.Blocks;
using SharpGLTF.Scenes;
using SharpGLTF.Schema2;

namespace AssetConverter
{
    /// <summary>
    /// Attaches the engine's node extras (particle systems, master particle systems, addon nodes, point lights) to the glTF
    /// scene of one NIF. Block names stay node names, so the node references inside particle extras keep resolving.
    /// Particle materials cannot hang on a mesh primitive, so they are created after the scene is built
    /// (<see cref="ResolveMaterials"/>); until then <c>particle_system.material</c> holds the material name.
    /// </summary>
    internal sealed class NifEffectNodes
    {
        public const string MissingReferenceCounter = "particle references to nodes missing from the glTF";
        public const string AmbiguousReferenceCounter = "particle references to node names used more than once in the glTF";

        private const string ParticleKey = "particle_system";
        private const string MasterKey = "master_particle_system";
        private const string AddonKey = "addon_node";
        private const string LightKey = "light";

        private readonly NifFile _nif;
        private readonly SceneBuilder _scene;
        private readonly Dictionary<int, NodeBuilder> _nodes;
        private readonly Dictionary<int, int> _parents;
        private readonly Dictionary<string, ParticleMaterial> _materials = new(StringComparer.Ordinal);

        public NifEffectNodes(NifFile nif, SceneBuilder scene, Dictionary<int, NodeBuilder> nodes)
        {
            _nif = nif;
            _scene = scene;
            _nodes = nodes;
            _parents = NifParentMap.Build(nif);
        }

        public void Attach(IReadOnlyList<SequenceClip> clips, IEffectShaderCatalog? catalog, Func<string, string> deployTexture, Action<string> reportWarning)
        {
            for (int i = 0; i < _nif.Blocks.Count; i++)
            {
                switch (_nif.Blocks[i])
                {
                    case NiParticleSystem system:
                        AttachParticleSystem(i, system, catalog, deployTexture, reportWarning);
                        break;
                    case BSMasterParticleSystem:
                        SetExtras(i, MasterKey, ParticleSystemExtras.BuildMaster(_nif, i));
                        break;
                    case BSValueNode:
                        SetExtras(i, AddonKey, NifNodeExtras.AddonNode(_nif, i));
                        break;
                    case NiPointLight:
                        AttachLight(i, clips);
                        break;
                }
            }
        }

        private void AttachParticleSystem(int block, NiParticleSystem system, IEffectShaderCatalog? catalog, Func<string, string> deployTexture, Action<string> reportWarning)
        {
            var material = ParticleMaterialConversion.TryBuild(_nif, system, catalog, deployTexture, reportWarning);
            if (material == null) return;
            var result = ParticleSystemExtras.Build(_nif, block, 0);
            if (result == null) return;

            NifAnimationStats.Add(ParticleMaterialConversion.WithMaterialCounter);
            _materials.TryAdd(material.Name, material);
            result.Extras["material"] = material.Name;
            SetExtras(block, ParticleKey, result.Extras);
        }

        private void AttachLight(int block, IReadOnlyList<SequenceClip> clips)
        {
            var result = NifNodeExtras.Light(_nif, block, clips, _parents);
            if (result.Extras != null)
                SetExtras(block, LightKey, result.Extras);
        }

        // Nodes that are not NiNodes (particle systems, lights) become glTF nodes of their own under their NIF parent.
        // A block without a parent sits at the scene root with an identity transform, like orphan shapes.
        private NodeBuilder NodeFor(int block)
        {
            if (_nodes.TryGetValue(block, out var existing))
                return existing;

            var obj = (NiAVObject)_nif.Blocks[block];
            var node = new NodeBuilder(NifNodeNames.Of(_nif, block));
            if (_parents.TryGetValue(block, out int parent) && _nodes.TryGetValue(parent, out var parentNode))
            {
                node.LocalTransform = GameCoordinates.ToGltfTransform(obj.Rotation, obj.Translation, obj.Scale);
                parentNode.AddNode(node);
            }
            else
            {
                _scene.AddNode(node);
            }
            _nodes[block] = node;
            return node;
        }

        private void SetExtras(int block, string key, JsonObject extras)
        {
            var node = NodeFor(block);
            var all = GltfNodeExtras.Existing(node.Extras);
            all[key] = extras;
            node.Extras = all;
        }

        /// <summary>Creates the particle materials in the model and points every <c>particle_system.material</c> at its glTF material index.</summary>
        public void ResolveMaterials(ModelRoot model)
        {
            var indices = new Dictionary<string, int>(StringComparer.Ordinal);
            foreach (var node in model.LogicalNodes)
            {
                if (node.Extras is not JsonObject all || all[ParticleKey] is not JsonObject system || system["material"] is not JsonValue name)
                    continue;
                string materialName = name.GetValue<string>();
                if (!indices.TryGetValue(materialName, out int index))
                {
                    index = MaterialIndex(model, _materials[materialName]);
                    indices[materialName] = index;
                }
                system["material"] = index;
            }
        }

        private static int MaterialIndex(ModelRoot model, ParticleMaterial particle)
        {
            var existing = model.LogicalMaterials.FirstOrDefault(m => m.Name == particle.Name);
            if (existing != null)
                return existing.LogicalIndex;

            var material = model.CreateMaterial(particle.Name);
            material.Extras = new JsonObject { [GltfMaterialExtras.EffectKind] = particle.Extras.DeepClone() };
            return material.LogicalIndex;
        }

        /// <summary>Counts particle node references (emitter objects, mesh-emitter sources, force objects) that do not resolve to exactly one glTF node.</summary>
        public static void VerifyReferences(ModelRoot model)
        {
            var counts = model.LogicalNodes.GroupBy(n => n.Name ?? "").ToDictionary(g => g.Key, g => g.Count(), StringComparer.Ordinal);
            foreach (var node in model.LogicalNodes)
            {
                if (node.Extras is not JsonObject all || all[ParticleKey] is not JsonObject system)
                    continue;
                foreach (string reference in References(system))
                {
                    if (reference.Length == 0) continue;
                    counts.TryGetValue(reference, out int found);
                    if (found == 0) NifAnimationStats.Add(MissingReferenceCounter);
                    else if (found > 1) NifAnimationStats.Add(AmbiguousReferenceCounter);
                }
            }
        }

        private static IEnumerable<string> References(JsonObject system)
        {
            foreach (var emitter in (system["emitters"] as JsonArray ?? new JsonArray()).OfType<JsonObject>())
            {
                if (emitter["node"] is JsonValue emitterNode) yield return emitterNode.GetValue<string>();
                foreach (var mesh in emitter["mesh_nodes"] as JsonArray ?? new JsonArray())
                    yield return mesh!.GetValue<string>();
            }
            foreach (var modifier in (system["modifiers"] as JsonArray ?? new JsonArray()).OfType<JsonObject>())
            {
                if (modifier["object"] is JsonValue forceObject) yield return forceObject.GetValue<string>();
            }
        }
    }
}
