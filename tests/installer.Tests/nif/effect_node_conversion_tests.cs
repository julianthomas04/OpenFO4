using System.Numerics;
using System.Reflection;
using System.Text.Json.Nodes;
using AssetConverter;
using InstallerTests.Archives;
using NiflySharp;
using NiflySharp.Bitfields;
using NiflySharp.Blocks;
using NiflySharp.Enums;
using NiflySharp.Structs;
using SharpGLTF.Schema2;
using Xunit;

namespace InstallerTests.Nif
{
    /// <summary>NIF -> GLB conversion of the node-level extras: particle systems, master particle systems, addon nodes and lights.</summary>
    [Collection(StaticStateCollections.NifConverterState)]
    public sealed class EffectNodeConversionTests : IDisposable
    {
        private const float UnsetValue = -3.4028235E+38f;
        private const string FireTexture = "textures\\Effects\\FireAtlas_d.dds";
        private const uint ParticleAtlasDescriptor = 0x8C;
        private static readonly FieldInfo MaxVertices = typeof(NiGeometryData).GetField("_bSMaxVertices", BindingFlags.NonPublic | BindingFlags.Instance)!;

        public EffectNodeConversionTests() => NifConverter.EffectCatalog = EffectCatalogFixture.With((ParticleAtlasDescriptor, ParticleAtlasDescriptor), (0x4, 0x10000004));

        public void Dispose() => NifConverter.EffectCatalog = null;

        [Fact]
        public void ConvertToGltf_ParticleOnlyMasterSystem_ProducesNodesWithParticleExtrasAndAnEffectMaterial()
        {
            var scene = new NifSceneBuilder();
            int master = AddMaster(scene, "FXMPSFire");
            int emitterNode = scene.Attach(scene.Root, Group("Emitter01"));
            int system = AddSystem(scene, "Fire01", emitterNode);
            ((BSMasterParticleSystem)scene.Nif.Blocks[master]).ParticleSystems.AddBlockRef(system);

            var model = Convert(scene);

            Assert.Empty(model.LogicalMeshes);
            var node = Node(model, "Fire01");
            var particle = node.Extras["particle_system"]!;
            Assert.Equal(1, particle["v"]!.GetValue<int>());
            Assert.Equal(240u, particle["max_particles"]!.GetValue<uint>());
            Assert.Equal("Emitter01", particle["emitters"]![0]!["node"]!.GetValue<string>());

            var material = model.LogicalMaterials[particle["material"]!.GetValue<int>()];
            var effect = material.Extras["effect"]!;
            Assert.Equal("effect_vs_0000008c", effect["vs"]!.GetValue<string>());
            Assert.Equal("effect_ps_0000008c", effect["ps"]!.GetValue<string>());
            Assert.Equal("effects/fireatlas_d.dds", effect["textures"]!["source"]!.GetValue<string>());
            Assert.Matches(@"^effects/fireatlas_d\.dds\|\|fx\|eff:[0-9a-f]{8}$", material.Name);

            var masterExtras = Node(model, "FXMPSFire").Extras["master_particle_system"]!;
            Assert.Equal(["Fire01"], masterExtras["systems"]!.AsArray().Select(s => s!.GetValue<string>()));
            Assert.NotNull(Node(model, "Emitter01"));
        }

        [Fact]
        public void ConvertToGltf_ParticleSystemUnderATransformedNode_KeepsItsLocalTransform()
        {
            var scene = new NifSceneBuilder();
            var holder = scene.AddNode(scene.Root, "Holder", new Vector3(30f, 0f, 0f));
            int emitterNode = scene.Attach(holder, Group("Emitter01"));
            AddSystem(scene, "Fire01", emitterNode, parent: holder, translation: new Vector3(0f, 0f, 20f));

            var model = Convert(scene);

            var node = Node(model, "Fire01");
            Assert.Same(Node(model, "Holder"), node.VisualParent);
            Assert.Equal(20f * GameCoordinates.Scaling, node.LocalTransform.Translation.Y, 1e-5f);
        }

        [Fact]
        public void ConvertToGltf_MeshEmitter_ExportsItsSourceMeshAsAnOrdinaryNodeNamedByMeshNodes()
        {
            var scene = new NifSceneBuilder();
            var triangle = new[] { new Vector3(0, 0, 0), new Vector3(10, 0, 0), new Vector3(0, 10, 0) };
            var source = scene.AddShape(scene.Root, "Flames01", triangle, [new Triangle(0, 1, 2)]);
            int sourceIndex = scene.Nif.Blocks.IndexOf(source);
            var emitter = new NiPSysMeshEmitter
            {
                Name = new NiStringRef("MeshEmitter"),
                EmitterMeshes = Pointers(sourceIndex),
                EmissionType = EmitFrom.EMIT_FROM_FACE_CENTER,
                InitialVelocityType = VelocityType.VELOCITY_USE_NORMALS,
            };
            AddSystem(scene, "Fire01", emitter: emitter);

            var model = Convert(scene);

            var particle = Node(model, "Fire01").Extras["particle_system"]!;
            Assert.Equal(["Flames01"], particle["emitters"]![0]!["mesh_nodes"]!.AsArray().Select(n => n!.GetValue<string>()));
            Assert.NotNull(Node(model, "Flames01").Mesh);
        }

        [Fact]
        public void ConvertToGltf_VertexEmitterSourceWithoutTriangles_IsExportedAsAPointsNodeUnderItsName()
        {
            var scene = new NifSceneBuilder();
            var points = new[] { new Vector3(0, 0, 0), new Vector3(10, 0, 0), new Vector3(0, 10, 0), new Vector3(0, 0, 10) };
            var source = scene.AddShape(scene.Root, "emitFireBall:0", points, []);
            var emitter = new NiPSysMeshEmitter
            {
                Name = new NiStringRef("MeshEmitter"),
                EmitterMeshes = Pointers(scene.Nif.Blocks.IndexOf(source)),
                EmissionType = EmitFrom.EMIT_FROM_VERTICES,
                InitialVelocityType = VelocityType.VELOCITY_USE_NORMALS,
            };
            AddSystem(scene, "Fire01", emitter: emitter);

            var model = Convert(scene);

            var node = Node(model, "emitFireBall:0");
            var primitive = Assert.Single(node.Mesh.Primitives);
            Assert.Equal(PrimitiveType.POINTS, primitive.DrawPrimitiveType);
            Assert.Equal(4, primitive.GetVertexAccessor("POSITION").Count);
            var emitterJson = Node(model, "Fire01").Extras["particle_system"]!["emitters"]![0]!;
            Assert.Equal("vertices", emitterJson["emission_type"]!.GetValue<string>());
            Assert.Equal(["emitFireBall:0"], emitterJson["mesh_nodes"]!.AsArray().Select(n => n!.GetValue<string>()));
            Assert.Equal("Fire01", emitterJson["node"]!.GetValue<string>());
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(NifConverter.EmitterSourcePointsCounter, StringComparison.Ordinal));
        }

        [Fact]
        public void ConvertToGltf_ShapeWithoutTrianglesThatNoEmitterReferences_IsNotExported()
        {
            var scene = new NifSceneBuilder();
            scene.AddShape(scene.Root, "StrayPoints", [new Vector3(0, 0, 0), new Vector3(10, 0, 0)], []);
            AddSystem(scene, "Fire01", scene.Nif.AddBlock(Group("Emitter01")));

            var model = Convert(scene);

            Assert.DoesNotContain(model.LogicalNodes, n => n.Name == "StrayPoints");
        }

        [Fact]
        public void ConvertToGltf_ParticleReferencingAMissingNode_IsCounted()
        {
            var scene = new NifSceneBuilder();
            var orphan = Group("NotInTheScene");
            int orphanIndex = scene.Nif.AddBlock(orphan);
            AddSystem(scene, "Fire01", orphanIndex);
            var model = Convert(scene);
            Assert.NotNull(Node(model, "Fire01"));

            // Orphan NiNodes become root nodes, so the reference resolves; a reference to a renamed node does not.
            Node(model, "NotInTheScene").Name = "Renamed";
            NifEffectNodes.VerifyReferences(model);

            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(NifEffectNodes.MissingReferenceCounter, StringComparison.Ordinal));
        }

        [Fact]
        public void ConvertToGltf_ParticleSystemWithoutACatalogue_IsLeftOutAndCounted()
        {
            NifConverter.EffectCatalog = null;
            var scene = new NifSceneBuilder();
            AddSystem(scene, "Fire01", scene.Nif.AddBlock(Group("Emitter01")));

            var model = Convert(scene);

            Assert.DoesNotContain(model.LogicalNodes, n => n.Extras?["particle_system"] != null);
            Assert.Contains(NifAnimationStats.EffectSummaryLines(), l => l.StartsWith(ParticleMaterialConversion.LeftOutPrefix + EffectCounters.NoCatalogReason, StringComparison.Ordinal));
        }

        [Fact]
        public void ConvertToGltf_ParticleSystemWithoutAnEffectShader_IsLeftOutAndCounted()
        {
            var scene = new NifSceneBuilder();
            int system = AddSystem(scene, "Fire01", scene.Nif.AddBlock(Group("Emitter01")));
            ((NiParticleSystem)scene.Nif.Blocks[system]).ShaderPropertyRef = new NiBlockRef<BSShaderProperty>(-1);

            var model = Convert(scene);

            Assert.DoesNotContain(model.LogicalNodes, n => n.Extras?["particle_system"] != null);
            Assert.Contains(NifAnimationStats.EffectSummaryLines(), l => l.StartsWith(ParticleMaterialConversion.LeftOutPrefix + ParticleMaterialConversion.NoEffectShaderReason, StringComparison.Ordinal));
        }

        [Fact]
        public void ConvertToGltf_ParticleSystemsWithTheSameShader_ShareOneMaterial()
        {
            var scene = new NifSceneBuilder();
            int emitterNode = scene.Attach(scene.Root, Group("Emitter01"));
            AddSystem(scene, "Fire01", emitterNode);
            AddSystem(scene, "Fire02", emitterNode);

            var model = Convert(scene);

            Assert.Single(model.LogicalMaterials);
            Assert.Equal(Node(model, "Fire01").Extras["particle_system"]!["material"]!.GetValue<int>(), Node(model, "Fire02").Extras["particle_system"]!["material"]!.GetValue<int>());
        }

        private static readonly Triangle[] OneTriangle = [new Triangle(0, 1, 2)];
        private static readonly Vector3[] TrianglePositions = [Vector3.Zero, Vector3.UnitX, Vector3.UnitY];

        [Fact]
        public void ConvertToGltf_NodeWithoutCulledFlag_HasNoHiddenExtra()
        {
            var scene = new NifSceneBuilder();
            var visible = scene.AddNode(scene.Root, "Visible", Vector3.Zero);
            scene.AddShape(visible, "VisibleShape", TrianglePositions, OneTriangle);

            var model = Convert(scene);

            Assert.Null(Node(model, "Visible").Extras?["hidden"]);
        }

        [Fact]
        public void ConvertToGltf_CulledNodeSubtree_MarksOnlyTheCulledNode()
        {
            var scene = new NifSceneBuilder();
            var hidden = scene.AddNode(scene.Root, "HiddenGroup", Vector3.Zero);
            hidden.Flags_ui = 0x2000000F;
            var child = scene.AddNode(hidden, "ChildOfHidden", Vector3.Zero);
            scene.AddShape(child, "InsideShape", TrianglePositions, OneTriangle);

            var model = Convert(scene);

            Assert.True(Node(model, "HiddenGroup").Extras["hidden"]!.GetValue<bool>());
            Assert.Null(Node(model, "ChildOfHidden").Extras?["hidden"]);
            Assert.Null(Node(model, "InsideShape").Extras?["hidden"]);
            Assert.NotNull(Node(model, "InsideShape").Mesh);
        }

        [Fact]
        public void ConvertToGltf_CulledShape_KeepsItsMeshAndMarksTheShapeNode()
        {
            var scene = new NifSceneBuilder();
            var shape = scene.AddShape(scene.Root, "Blood", TrianglePositions, OneTriangle);
            shape.Flags_ui = 1;

            var model = Convert(scene);

            var node = Node(model, "Blood");
            Assert.True(node.Extras["hidden"]!.GetValue<bool>());
            Assert.NotNull(node.Mesh);
        }

        [Fact]
        public void ConvertToGltf_CulledParticleSystem_MarksTheParticleNode()
        {
            var scene = new NifSceneBuilder();
            int emitterNode = scene.Attach(scene.Root, Group("Emitter01"));
            int system = AddSystem(scene, "Fire01", emitterNode);
            ((NiParticleSystem)scene.Nif.Blocks[system]).Flags_ui = 1;

            var model = Convert(scene);

            var node = Node(model, "Fire01");
            Assert.True(node.Extras["hidden"]!.GetValue<bool>());
            Assert.NotNull(node.Extras["particle_system"]);
        }

        [Fact]
        public void ConvertToGltf_DamageStageNode_CarriesRangeMinAndMaxButNotCurrent()
        {
            var scene = new NifSceneBuilder();
            var stage = new BSDamageStage { Children = new NiBlockRefArray<NiAVObject>(), Name = new NiStringRef("DamageStage1"), Min = 1, Max = 3, Current = 2, Rotation = NifSceneBuilder.Identity(), Scale = 1f, Flags_ui = 0x2000000F };
            scene.Attach(scene.Root, stage);

            var model = Convert(scene);

            var extras = Node(model, "DamageStage1").Extras;
            Assert.Equal(1, extras["damage_stage"]!["min"]!.GetValue<int>());
            Assert.Equal(3, extras["damage_stage"]!["max"]!.GetValue<int>());
            Assert.Equal(["min", "max"], extras["damage_stage"]!.AsObject().Select(p => p.Key));
            Assert.True(extras["hidden"]!.GetValue<bool>());
        }

        [Fact]
        public void ConvertToGltf_PlainNode_HasNoDamageStage()
        {
            var scene = new NifSceneBuilder();
            var plain = scene.AddNode(scene.Root, "Plain", Vector3.Zero);
            scene.AddShape(plain, "PlainShape", TrianglePositions, OneTriangle);

            Assert.Null(Node(Convert(scene), "Plain").Extras?["damage_stage"]);
        }

        [Fact]
        public void ConvertToGltf_AddonMarker_CarriesTheAddonNodeExtrasAtItsTransform()
        {
            var scene = new NifSceneBuilder();
            var marker = new BSValueNode { Children = new NiBlockRefArray<NiAVObject>(), Name = new NiStringRef("FXFireMed01"), Value = 5, ValueNodeFlags = 0, Translation = new Vector3(0f, 0f, 40f), Rotation = NifSceneBuilder.Identity(), Scale = 1f };
            scene.Attach(scene.Root, marker);

            var model = Convert(scene);

            var node = Node(model, "FXFireMed01");
            var addon = node.Extras["addon_node"]!;
            Assert.Equal(5, addon["index"]!.GetValue<int>());
            Assert.Equal(0u, addon["flags"]!.GetValue<uint>());
            Assert.Equal(40f * GameCoordinates.Scaling, node.LocalTransform.Translation.Y, 1e-5f);
        }

        [Fact]
        public void ConvertToGltf_AnimatedPointLight_CarriesTheLightExtrasWithItsDimmerTrack()
        {
            var scene = new NifSceneBuilder();
            var light = new NiPointLight
            {
                Name = new NiStringRef("FlickerLight"),
                Dimmer = 2.5f,
                Rotation = NifSceneBuilder.Identity(),
                Scale = 1f,
                Controller = new NiBlockRef<NiTimeController>(-1),
            };
            int lightIndex = scene.Attach(scene.Root, light);
            AddDimmerController(scene.Nif, lightIndex, (0f, 2.5f), (1f, 1.5f), (2f, 2.5f));

            var model = Convert(scene);

            var extras = Node(model, "FlickerLight").Extras["light"]!;
            Assert.Equal("point", extras["kind"]!.GetValue<string>());
            Assert.Equal(2.5f, extras["dimmer_rest"]!.GetValue<float>());
            Assert.Equal("dimmer", extras["tracks"]![0]!["target"]!.GetValue<string>());
        }

        [Fact]
        public void ConvertToGltf_StaticPointLight_GetsNoNode()
        {
            var scene = new NifSceneBuilder();
            scene.Attach(scene.Root, new NiPointLight { Name = new NiStringRef("Steady"), Dimmer = 1f, Rotation = NifSceneBuilder.Identity(), Scale = 1f, Controller = new NiBlockRef<NiTimeController>(-1) });

            var model = Convert(scene);

            Assert.DoesNotContain(model.LogicalNodes, n => n.Name == "Steady");
        }

        [Fact]
        public void ConvertToGltf_BarrelWithEffectAndRefractionShapes_GivesBothMaterialsTheirExtras()
        {
            var scene = new NifSceneBuilder();
            var triangle = new[] { new Vector3(0, 0, 0), new Vector3(10, 0, 0), new Vector3(0, 10, 0) };
            var uvs = new[] { new TexCoord(0f, 0f), new TexCoord(1f, 0f), new TexCoord(0f, 1f) };
            var mist = scene.AddShape(scene.Root, "Mist", triangle, [new Triangle(0, 1, 2)], uvs);
            Unskin(mist);
            scene.AttachShader(mist, EffectShaderFixture.Create(2348810240, 48, "textures\\Effects\\MistTileLight_d.dds"));
            mist.AlphaPropertyRef = new NiBlockRef<NiAlphaProperty>(scene.Nif.AddBlock(new NiAlphaProperty { Flags = new AlphaFlags { Value = 4333 }, Threshold = 128 }));
            var haze = scene.AddShape(scene.Root, "ref003:0", triangle, [new Triangle(0, 1, 2)], uvs);
            Unskin(haze);
            int textures = scene.Nif.AddBlock(new BSShaderTextureSet
            {
                Textures = new[] { "textures\\Effects\\SmokeVapor02Tile_n.dds", "textures\\Effects\\SmokeVapor01Tile_n.dds", "", "", "", "", "", "", "", "" }
                    .Select(path => new NiString4(path, false)).ToList(),
            });
            scene.AttachShader(haze, EffectShaderFixture.CreateRefraction(0x80008001u, 33, 0.04f, textures));

            var model = Convert(scene);

            Assert.Equal(2, model.LogicalMaterials.Count);
            Assert.Single(model.LogicalMaterials, m => m.Extras?["effect"] != null);
            Assert.Single(model.LogicalMaterials, m => m.Extras?["refraction"] != null);
            Assert.Contains(NifAnimationStats.EffectSummaryLines(), l => l.StartsWith(EffectCounters.WithExtras, StringComparison.Ordinal));
            Assert.Contains(NifAnimationStats.EffectSummaryLines(), l => l.StartsWith(EffectCounters.RefractionShapes, StringComparison.Ordinal));
        }

        [Fact]
        public void SummaryLines_EffectCountersAreSeparatedFromAnimationCounters()
        {
            NifAnimationStats.Add(ParticleSystemExtras.ModifierSkippedCounter);
            NifAnimationStats.Add(NifNodeExtras.OffsetTiesCounter);
            NifAnimationStats.Add(ShaderControllerTracks.WithoutKeysCounter);
            NifAnimationStats.AddUnsupported("particle modifier NiPSysBombModifier");
            NifAnimationStats.Add(NifAnimationStats.SequencesExported);

            var effects = NifAnimationStats.EffectSummaryLines().ToList();
            var animation = NifAnimationStats.AnimationSummaryLines().ToList();

            Assert.Contains(effects, l => l.StartsWith(ParticleSystemExtras.ModifierSkippedCounter, StringComparison.Ordinal));
            Assert.Contains(effects, l => l.StartsWith(NifNodeExtras.OffsetTiesCounter, StringComparison.Ordinal));
            Assert.Contains(effects, l => l.StartsWith(ShaderControllerTracks.WithoutKeysCounter, StringComparison.Ordinal));
            Assert.Contains(effects, l => l.StartsWith(NifAnimationStats.UnsupportedPrefix + "particle modifier", StringComparison.Ordinal));
            Assert.DoesNotContain(animation, l => effects.Contains(l));
            Assert.Contains(animation, l => l.StartsWith(NifAnimationStats.SequencesExported, StringComparison.Ordinal));
        }

        private static NiNode Group(string name) =>
            new() { Name = new NiStringRef(name), Children = new NiBlockRefArray<NiAVObject>(), Rotation = NifSceneBuilder.Identity(), Scale = 1f };

        private static void Unskin(BSTriShape shape)
        {
            shape.SkinInstanceRef = new NiBlockRef<NiObject>(-1);
            shape.VertexDesc.Value &= ~(0x40UL << 44);
        }

        private static int AddMaster(NifSceneBuilder scene, string name) =>
            scene.Attach(scene.Root, new BSMasterParticleSystem
            {
                Name = new NiStringRef(name),
                MaxEmitterObjects = 20,
                ParticleSystems = new NiBlockRefArray<NiAVObject>(),
                Children = new NiBlockRefArray<NiAVObject>(),
                Rotation = NifSceneBuilder.Identity(),
                Scale = 1f,
            });

        // A cylinder-emitter fire system with a 2x2 flipbook, drawn by an effect shader with the atlas texture.
        private static int AddSystem(NifSceneBuilder scene, string name, int emitterNode = -1, NiNode? parent = null, Vector3? translation = null, NiPSysEmitter? emitter = null)
        {
            var nif = scene.Nif;
            var data = new NiPSysData
            {
                AspectRatio = 1f,
                HasTextureIndices = true,
                SubtextureOffsets = [new(0, 0.5f, 0, 0.5f), new(0.5f, 0.5f, 0, 0.5f), new(0, 0.5f, 0.5f, 0.5f), new(0.5f, 0.5f, 0.5f, 0.5f)],
            };
            MaxVertices.SetValue(data, (ushort)240);
            int shader = nif.AddBlock(EffectShaderFixture.Create(0, 0, FireTexture));
            var system = new NiParticleSystem
            {
                Name = new NiStringRef(name),
                WorldSpace = true,
                Translation = translation ?? Vector3.Zero,
                Rotation = NifSceneBuilder.Identity(),
                Scale = 1f,
                Data_NiBlockRef_NPSD = new NiBlockRef<NiPSysData>(nif.AddBlock(data)),
                ShaderPropertyRef = new NiBlockRef<BSShaderProperty>(shader),
                AlphaPropertyRef = new NiBlockRef<NiAlphaProperty>(-1),
                Controller = new NiBlockRef<NiTimeController>(-1),
                Modifiers = new NiBlockRefArray<NiPSysModifier>(),
            };
            emitter ??= new NiPSysCylinderEmitter
            {
                Name = new NiStringRef("Emitter"),
                EmitterObject = new NiBlockPtr<NiNode>(emitterNode),
                Radius = 22.5f,
                Height = 8f,
                Speed = 90f,
                InitialColor = new Color4 { R = 1, G = 1, B = 1, A = 1 },
                InitialRadius = 5f,
                LifeSpan = 1.3f,
            };
            system.Modifiers.AddBlockRef(nif.AddBlock(emitter));
            system.Modifiers.AddBlockRef(nif.AddBlock(new NiPSysAgeDeathModifier { Name = new NiStringRef("AgeDeath") }));
            return scene.Attach(parent ?? scene.Root, system);
        }

        private static NiBlockPtrArray<NiAVObject> Pointers(params int[] indices)
        {
            var array = new NiBlockPtrArray<NiAVObject>();
            foreach (int index in indices)
                array.AddBlockRef(index);
            return array;
        }

        private static void AddDimmerController(NifFile nif, int light, params (float Time, float Value)[] keys)
        {
            var data = new NiFloatData
            {
                Data = new KeyGroup<float>
                {
                    Interpolation = KeyType.LINEAR_KEY,
                    NumKeys = (uint)keys.Length,
                    Keys = keys.Select(k => new Key<float> { Time = k.Time, Value = k.Value }).ToList(),
                },
            };
            int interpolator = nif.AddBlock(new NiFloatInterpolator { Value = UnsetValue, Data = new NiBlockRef<NiFloatData>(nif.AddBlock(data)) });
            var controller = new NiLightDimmerController
            {
                StartTime = 0f,
                StopTime = keys[^1].Time,
                Frequency = 1f,
                Flags = new TimeControllerFlags { CycleType = CycleType.CYCLE_LOOP },
                NextController = new NiBlockRef<NiTimeController>(-1),
                Interpolator = new NiBlockRef<NiInterpolator>(interpolator),
            };
            ((NiObjectNET)nif.Blocks[light]).Controller = new NiBlockRef<NiTimeController>(nif.AddBlock(controller));
        }

        private static Node Node(ModelRoot model, string name) => model.LogicalNodes.Single(n => n.Name == name);

        private static ModelRoot Convert(NifSceneBuilder scene)
        {
            using var dir = new ArchiveTempDir();
            string source = scene.Save(dir.Path, "model.nif");
            string destination = Path.Combine(dir.Path, "model.glb");
            NifConverter.ConvertToGltf(source, destination);
            return ModelRoot.Load(destination);
        }
    }
}
