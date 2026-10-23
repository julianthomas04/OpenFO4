using System.Numerics;
using System.Text.Json.Nodes;
using AssetConverter;
using NiflySharp;
using NiflySharp.Bitfields;
using NiflySharp.Blocks;
using NiflySharp.Enums;
using NiflySharp.Structs;
using Xunit;

namespace InstallerTests.Nif
{
    [Collection(StaticStateCollections.NifConverterState)]
    public class NifNodeExtrasTests
    {
        private const float Tolerance = 1e-4f;
        private const float UnsetValue = -3.4028235E+38f;

        [Fact]
        public void AddonNode_FxFireMarker_EmitsIndexAndFlags()
        {
            var scene = new NodeExtrasScene();
            int marker = scene.AddBlock(scene.Root, new BSValueNode { Name = new NiStringRef("FXFireMed01"), Value = 5, ValueNodeFlags = 0 });

            var extras = NifNodeExtras.AddonNode(scene.Nif, marker);

            Assert.Equal(1, extras["v"]!.GetValue<int>());
            Assert.Equal(5, extras["index"]!.GetValue<int>());
            Assert.Equal(0u, extras["flags"]!.GetValue<uint>());
            Assert.Equal(new[] { "v", "index", "flags" }, extras.Select(p => p.Key));
        }

        [Fact]
        public void AddonNode_CarriesTheValueNodeFlags()
        {
            var scene = new NodeExtrasScene();
            int marker = scene.AddBlock(scene.Root, new BSValueNode { Name = new NiStringRef("Marker"), Value = 12, ValueNodeFlags = BSValueNodeFlags.UsePlayerAdjust });

            var extras = NifNodeExtras.AddonNode(scene.Nif, marker);

            Assert.Equal(12, extras["index"]!.GetValue<int>());
            Assert.Equal((uint)BSValueNodeFlags.UsePlayerAdjust, extras["flags"]!.GetValue<uint>());
        }

        [Fact]
        public void AddonNode_PlainNode_Throws()
        {
            var scene = new NodeExtrasScene();
            int plain = scene.AddBlock(scene.Root, new NiNode { Name = new NiStringRef("Plain") });

            Assert.Throws<NifAnimationException>(() => NifNodeExtras.AddonNode(scene.Nif, plain));
            Assert.Throws<NifAnimationException>(() => NifNodeExtras.AddonNode(scene.Nif, 999));
        }

        [Fact]
        public void Light_FlickerLightShape_HasDimmerAndOffsetTracksInEngineUnits()
        {
            var scene = new NodeExtrasScene();
            int mover = scene.AddBlock(scene.Root, new NiNode { Name = new NiStringRef("FlickerMover"), Translation = new Vector3(0f, 0f, 100f), Rotation = NifSceneBuilder.Identity(), Scale = 1f });
            int light = scene.AddLight(mover, new NiPointLight { Name = new NiStringRef("FlickerLight"), Dimmer = 2.5f });
            scene.AddDimmerController(light, new ScalarKey(0f, 2.5f), new ScalarKey(2f, 1.5f));
            scene.AddLoopingSequence("Flicker", 0f, 2f, "FlickerMover", new VectorKey(0f, new Vector3(0f, 0f, 100f)), new VectorKey(1f, new Vector3(10f, 0f, 120f)), new VectorKey(2f, new Vector3(0f, 0f, 100f)));

            var result = NifNodeExtras.Light(scene.Nif, light, scene.Clips());

            var extras = result.Extras!;
            Assert.Equal(1, extras["v"]!.GetValue<int>());
            Assert.Equal("point", extras["kind"]!.GetValue<string>());
            Assert.Equal(2.5f, extras["dimmer_rest"]!.GetValue<float>());
            var tracks = extras["tracks"]!.AsArray();
            Assert.Equal(new[] { "dimmer", "offset" }, tracks.Select(t => t!["target"]!.GetValue<string>()));

            var dimmer = tracks[0]!;
            Assert.Equal("loop", dimmer["cycle"]!.GetValue<string>());
            Assert.Equal(2.5f, dimmer["values"]![0]!.GetValue<float>());

            var offset = tracks[1]!;
            Assert.Equal("loop", offset["cycle"]!.GetValue<string>());
            Assert.Equal(0f, offset["start"]!.GetValue<float>());
            Assert.Equal(2f, offset["stop"]!.GetValue<float>());
            var times = offset["times"]!.AsArray();
            Assert.Equal(61, times.Count);
            Assert.Equal(2f, times[times.Count - 1]!.GetValue<float>());
            AssertVector(new Vector3(0f, 0f, 0f), offset["values"]![0]!);
            // Game (10, 0, 20) -> glTF (1, 2, 0): x and z swap axes and shrink by 0.1.
            AssertVector(new Vector3(1f, 2f, 0f), offset["values"]![30]!);
            AssertVector(new Vector3(0f, 0f, 0f), offset["values"]![60]!);
            Assert.Empty(result.Unsupported);
        }

        [Fact]
        public void Light_ParentRotatingAboutZ_MovesTheLightAlongItsOrbit()
        {
            var scene = new NodeExtrasScene();
            int pivot = scene.AddBlock(scene.Root, new NiNode { Name = new NiStringRef("Pivot"), Rotation = NifSceneBuilder.Identity(), Scale = 1f });
            int light = scene.AddLight(pivot, new NiPointLight { Name = new NiStringRef("Orbiting"), Translation = new Vector3(10f, 0f, 0f), Dimmer = 1f });
            scene.AddLoopingSequence("Spin", 0f, 1f, "Pivot", new RotationKey(0f, AboutZ(0f)), new RotationKey(1f, AboutZ(90f)));

            var offset = Assert.Single(NifNodeExtras.Light(scene.Nif, light, scene.Clips()).Extras!["tracks"]!.AsArray())!;

            Assert.Equal("offset", offset["target"]!.GetValue<string>());
            AssertVector(Vector3.Zero, offset["values"]![0]!);
            // Game position (10, 0, 0) turns to (0, 10, 0): glTF (1, 0, 0) -> (0, 0, -1).
            AssertVector(new Vector3(-1f, 0f, -1f), Last(offset["values"]!));
        }

        [Fact]
        public void Light_KeyedRotationEqualToTheRestRotation_AddsNoOffsetTrack()
        {
            var scene = new NodeExtrasScene();
            int pivot = scene.AddBlock(scene.Root, new NiNode { Name = new NiStringRef("Pivot"), Rotation = NifSceneBuilder.AboutZ(90f), Scale = 1f });
            int light = scene.AddLight(pivot, new NiPointLight { Name = new NiStringRef("Steady"), Translation = new Vector3(10f, 0f, 0f), Dimmer = 1f });
            scene.AddLoopingSequence("Hold", 0f, 1f, "Pivot", new RotationKey(0f, AboutZ(90f)), new RotationKey(1f, AboutZ(90f)));

            var result = NifNodeExtras.Light(scene.Nif, light, scene.Clips());

            Assert.Null(result.Extras);
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(NifNodeExtras.OffsetAllZeroCounter, StringComparison.Ordinal));
        }

        [Fact]
        public void Light_RootNodeMotion_DoesNotMoveTheLightBecauseTheConverterStripsTheRoot()
        {
            var scene = new NodeExtrasScene();
            int light = scene.AddLight(scene.RootIndex, new NiPointLight { Name = new NiStringRef("OnRoot"), Dimmer = 1f });
            scene.AddLoopingSequence("RootMoves", 0f, 1f, "Scene", new VectorKey(0f, Vector3.Zero), new VectorKey(1f, new Vector3(50f, 0f, 0f)));

            Assert.Null(NifNodeExtras.Light(scene.Nif, light, scene.Clips()).Extras);
        }

        [Fact]
        public void Light_SeveralSequences_TheLoopingOneWinsAndTiesAreCounted()
        {
            var scene = new NodeExtrasScene();
            int mover = scene.AddBlock(scene.Root, new NiNode { Name = new NiStringRef("Mover"), Rotation = NifSceneBuilder.Identity(), Scale = 1f });
            int light = scene.AddLight(mover, new NiPointLight { Name = new NiStringRef("L"), Dimmer = 1f });
            scene.AddSequence("clamped", CycleType.CYCLE_CLAMP, 0f, 1f, "Mover", new VectorKey(0f, Vector3.Zero), new VectorKey(1f, new Vector3(100f, 0f, 0f)));
            scene.AddSequence("loopA", CycleType.CYCLE_LOOP, 0f, 1f, "Mover", new VectorKey(0f, Vector3.Zero), new VectorKey(1f, new Vector3(10f, 0f, 0f)));
            scene.AddSequence("loopB", CycleType.CYCLE_LOOP, 0f, 1f, "Mover", new VectorKey(0f, Vector3.Zero), new VectorKey(1f, new Vector3(20f, 0f, 0f)));

            var offset = Assert.Single(NifNodeExtras.Light(scene.Nif, light, scene.Clips()).Extras!["tracks"]!.AsArray())!;

            Assert.Equal("loop", offset["cycle"]!.GetValue<string>());
            AssertVector(new Vector3(1f, 0f, 0f), Last(offset["values"]!));
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(NifNodeExtras.OffsetTiesCounter, StringComparison.Ordinal));
        }

        [Fact]
        public void Light_AncestorWithStandaloneTransformController_DrivesTheOffsetWithTheControllersTiming()
        {
            var scene = new NodeExtrasScene();
            int mover = scene.AddAnimatedNode(scene.Root, "Omni001");
            int light = scene.AddLight(mover, new NiPointLight { Name = new NiStringRef("Flicker"), Dimmer = 1f });
            scene.AddStandaloneTransform(mover, CycleType.CYCLE_LOOP, 0f, 2f, 2f, 0.5f,
                new VectorKey(0f, Vector3.Zero), new VectorKey(1f, new Vector3(10f, 0f, 0f)), new VectorKey(2f, Vector3.Zero));

            var offset = Assert.Single(NifNodeExtras.Light(scene.Nif, light, scene.Clips()).Extras!["tracks"]!.AsArray())!;

            Assert.Equal("offset", offset["target"]!.GetValue<string>());
            Assert.Equal("loop", offset["cycle"]!.GetValue<string>());
            Assert.Equal((0f, 2f, 2f, 0.5f), (offset["start"]!.GetValue<float>(), offset["stop"]!.GetValue<float>(), offset["frequency"]!.GetValue<float>(), offset["phase"]!.GetValue<float>()));
            Assert.Equal(61, offset["times"]!.AsArray().Count);
            AssertVector(new Vector3(1f, 0f, 0f), offset["values"]![30]!);
        }

        [Fact]
        public void Light_StandaloneControllerWithEmptyRange_UsesTheKeyRangeAndCountsIt()
        {
            var scene = new NodeExtrasScene();
            int mover = scene.AddAnimatedNode(scene.Root, "Omni001");
            int light = scene.AddLight(mover, new NiPointLight { Name = new NiStringRef("Flicker"), Dimmer = 1f });
            scene.AddStandaloneTransform(mover, CycleType.CYCLE_LOOP, 0f, 0f, 1f, 0f, new VectorKey(0f, Vector3.Zero), new VectorKey(1f, new Vector3(10f, 0f, 0f)));

            var offset = Assert.Single(NifNodeExtras.Light(scene.Nif, light, scene.Clips()).Extras!["tracks"]!.AsArray())!;

            Assert.Equal((0f, 1f), (offset["start"]!.GetValue<float>(), offset["stop"]!.GetValue<float>()));
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(ClipTiming.RangeReplacedCounter, StringComparison.Ordinal));
        }

        [Fact]
        public void Light_StandaloneControllersWithDifferentTiming_KeepOneGroupAndCountTheDroppedOne()
        {
            var scene = new NodeExtrasScene();
            int outer = scene.AddAnimatedNode(scene.Root, "Outer");
            int inner = scene.AddAnimatedNode(outer, "Inner");
            int light = scene.AddLight(inner, new NiPointLight { Name = new NiStringRef("L"), Dimmer = 1f });
            scene.AddStandaloneTransform(outer, CycleType.CYCLE_CLAMP, 0f, 1f, 1f, 0f, new VectorKey(0f, Vector3.Zero), new VectorKey(1f, new Vector3(100f, 0f, 0f)));
            scene.AddStandaloneTransform(inner, CycleType.CYCLE_LOOP, 0f, 1f, 1f, 0f, new VectorKey(0f, Vector3.Zero), new VectorKey(1f, new Vector3(10f, 0f, 0f)));

            var offset = Assert.Single(NifNodeExtras.Light(scene.Nif, light, scene.Clips()).Extras!["tracks"]!.AsArray())!;

            Assert.Equal("loop", offset["cycle"]!.GetValue<string>());
            AssertVector(new Vector3(1f, 0f, 0f), Last(offset["values"]!));
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(NifNodeExtras.OffsetDroppedStandaloneCounter, StringComparison.Ordinal));
        }

        [Fact]
        public void Light_AncestorControllerDrivenBySequence_IsNotTakenForAStandaloneController()
        {
            var scene = new NodeExtrasScene();
            int mover = scene.AddAnimatedNode(scene.Root, "Mover");
            int light = scene.AddLight(mover, new NiPointLight { Name = new NiStringRef("L"), Dimmer = 1f });
            scene.AddTransformController(mover);

            Assert.Null(NifNodeExtras.Light(scene.Nif, light, scene.Clips()).Extras);
        }

        [Fact]
        public void Light_DiffuseColorController_IsEmittedAsThreeComponentLightColor()
        {
            var scene = new NodeExtrasScene();
            int light = scene.AddLight(scene.RootIndex, new NiPointLight { Name = new NiStringRef("Tinted"), Dimmer = 1f });
            scene.AddDiffuseColorController(light, new Vector3(1f, 0.5f, 0.25f), new Vector3(0f, 0f, 0f));

            var track = Assert.Single(NifNodeExtras.Light(scene.Nif, light, Array.Empty<SequenceClip>()).Extras!["tracks"]!.AsArray())!;

            Assert.Equal("light_color", track["target"]!.GetValue<string>());
            AssertVector(new Vector3(1f, 0.5f, 0.25f), track["values"]![0]!);
        }

        [Fact]
        public void Light_StaticLight_HasNoExtras()
        {
            var scene = new NodeExtrasScene();
            int light = scene.AddLight(scene.RootIndex, new NiPointLight { Name = new NiStringRef("Static"), Dimmer = 1f });

            var result = NifNodeExtras.Light(scene.Nif, light, Array.Empty<SequenceClip>());

            Assert.Null(result.Extras);
            Assert.Empty(result.Unsupported);
        }

        [Fact]
        public void Light_SpotLight_IsCountedUnsupported()
        {
            var scene = new NodeExtrasScene();
            int light = scene.AddLight(scene.RootIndex, new NiSpotLight { Name = new NiStringRef("Spot"), Dimmer = 1f });

            var result = NifNodeExtras.Light(scene.Nif, light, Array.Empty<SequenceClip>());

            Assert.Null(result.Extras);
            Assert.Equal("light block NiSpotLight", Assert.Single(result.Unsupported));
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(NifAnimationStats.UnsupportedPrefix + "light block NiSpotLight: ", StringComparison.Ordinal));
        }

        [Fact]
        public void Light_TransformControllerOnTheLight_IsReportedUnsupported()
        {
            var scene = new NodeExtrasScene();
            int light = scene.AddLight(scene.RootIndex, new NiPointLight { Name = new NiStringRef("SelfMoving"), Dimmer = 1f });
            scene.AddDimmerController(light, new ScalarKey(0f, 1f), new ScalarKey(1f, 2f));
            scene.AddTransformController(light);

            var result = NifNodeExtras.Light(scene.Nif, light, Array.Empty<SequenceClip>());

            Assert.Equal("NiTransformController on the light block", Assert.Single(result.Unsupported));
            Assert.Equal("dimmer", Assert.Single(result.Extras!["tracks"]!.AsArray())!["target"]!.GetValue<string>());
        }

        [Fact]
        public void Light_NotALight_Throws()
        {
            var scene = new NodeExtrasScene();

            Assert.Throws<NifAnimationException>(() => NifNodeExtras.Light(scene.Nif, scene.RootIndex, Array.Empty<SequenceClip>()));
        }

        private static Quaternion AboutZ(float degrees)
        {
            // NiflySharp stores NIF quaternions (w, x, y, z) into X, Y, Z, W in file order.
            float half = degrees * MathF.PI / 360f;
            return new Quaternion(MathF.Cos(half), 0f, 0f, MathF.Sin(half));
        }

        private static JsonNode Last(JsonNode array) => array.AsArray()[array.AsArray().Count - 1]!;

        private static void AssertVector(Vector3 expected, JsonNode actual)
        {
            var values = actual.AsArray().Select(v => v!.GetValue<float>()).ToArray();
            Assert.Equal(3, values.Length);
            Assert.Equal(expected.X, values[0], Tolerance);
            Assert.Equal(expected.Y, values[1], Tolerance);
            Assert.Equal(expected.Z, values[2], Tolerance);
        }

        private readonly record struct ScalarKey(float Time, float Value);

        private readonly record struct VectorKey(float Time, Vector3 Value);

        private readonly record struct RotationKey(float Time, Quaternion Value);

        private sealed class NodeExtrasScene
        {
            private readonly NifSceneBuilder _builder = new();

            public NifFile Nif => _builder.Nif;

            public NiNode Root => _builder.Root;

            public int RootIndex => Nif.Blocks.IndexOf(Root);

            public int AddBlock(NiNode parent, NiAVObject child)
            {
                int index = Nif.AddBlock(child);
                parent.Children ??= new NiBlockRefArray<NiAVObject>();
                parent.Children.AddBlockRef(index);
                return index;
            }

            public int AddBlock(int parent, NiAVObject child) => AddBlock((NiNode)Nif.Blocks[parent], child);

            public int AddLight(int parent, NiLight light)
            {
                light.Rotation = NifSceneBuilder.Identity();
                light.Scale = 1f;
                light.Controller = new NiBlockRef<NiTimeController>(-1);
                return AddBlock(parent, light);
            }

            public int AddAnimatedNode(NiNode parent, string name)
                => AddBlock(parent, new NiNode { Name = new NiStringRef(name), Rotation = NifSceneBuilder.Identity(), Scale = 1f, Controller = new NiBlockRef<NiTimeController>(-1) });

            public int AddAnimatedNode(int parent, string name) => AddAnimatedNode((NiNode)Nif.Blocks[parent], name);

            public void AddStandaloneTransform(int node, CycleType cycle, float start, float stop, float frequency, float phase, params VectorKey[] keys)
            {
                var interpolator = new NiTransformInterpolator
                {
                    Transform = new NiQuatTransform { Translation = new Vector3(UnsetValue), Rotation = new Quaternion(UnsetValue, UnsetValue, UnsetValue, UnsetValue), Scale = UnsetValue },
                    Data = new NiBlockRef<NiTransformData>(Nif.AddBlock(TranslationData(keys))),
                };
                AddController<NiTransformController>(node, Nif.AddBlock(interpolator), stop, c =>
                {
                    c.StartTime = start;
                    c.Frequency = frequency;
                    c.Phase = phase;
                    c.Flags.CycleType = cycle;
                });
            }

            public IReadOnlyList<SequenceClip> Clips() => NifControllerExtractor.Extract(Nif, _ => { });

            public void AddDimmerController(int light, params ScalarKey[] keys)
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
                int interpolator = Nif.AddBlock(new NiFloatInterpolator { Value = UnsetValue, Data = new NiBlockRef<NiFloatData>(Nif.AddBlock(data)) });
                AddController<NiLightDimmerController>(light, interpolator, keys[^1].Time, c => c.Flags.CycleType = CycleType.CYCLE_LOOP);
            }

            public void AddDiffuseColorController(int light, Vector3 from, Vector3 to)
            {
                var data = new NiPosData
                {
                    Data = new KeyGroup<Vector3>
                    {
                        Interpolation = KeyType.LINEAR_KEY,
                        NumKeys = 2,
                        Keys = new List<Key<Vector3>> { new() { Time = 0f, Value = from }, new() { Time = 1f, Value = to } },
                    },
                };
                int interpolator = Nif.AddBlock(new NiPoint3Interpolator { Value = new Vector3(UnsetValue), Data = new NiBlockRef<NiPosData>(Nif.AddBlock(data)) });
                AddController<NiLightColorController>(light, interpolator, 1f, c => c.TargetColor = LightColor.LC_DIFFUSE);
            }

            public void AddTransformController(int light)
                => AddController<NiTransformController>(light, Nif.AddBlock(new NiBlendFloatInterpolator { Value = UnsetValue }), 1f);

            private void AddController<T>(int owner, int interpolator, float stop, Action<T>? configure = null) where T : NiSingleInterpController, new()
            {
                var controller = new T
                {
                    StartTime = 0f,
                    StopTime = stop,
                    Frequency = 1f,
                    Flags = new TimeControllerFlags(),
                    NextController = new NiBlockRef<NiTimeController>(-1),
                    Interpolator = new NiBlockRef<NiInterpolator>(interpolator),
                };
                configure?.Invoke(controller);
                int index = Nif.AddBlock(controller);
                var ownerBlock = (NiObjectNET)Nif.Blocks[owner];
                if (ownerBlock.Controller.Index < 0)
                {
                    ownerBlock.Controller = new NiBlockRef<NiTimeController>(index);
                    return;
                }
                var tail = (NiTimeController)Nif.Blocks[ownerBlock.Controller.Index];
                while (tail.NextController.Index >= 0)
                    tail = (NiTimeController)Nif.Blocks[tail.NextController.Index];
                tail.NextController = new NiBlockRef<NiTimeController>(index);
            }

            public void AddLoopingSequence(string name, float start, float stop, string node, params VectorKey[] keys)
                => AddSequence(name, CycleType.CYCLE_LOOP, start, stop, node, keys);

            public void AddLoopingSequence(string name, float start, float stop, string node, params RotationKey[] keys)
                => AddSequence(name, CycleType.CYCLE_LOOP, start, stop, node, keys);

            public void AddSequence(string name, CycleType cycle, float start, float stop, string node, params VectorKey[] keys)
                => AddSequenceData(name, cycle, start, stop, node, TranslationData(keys));

            public void AddSequence(string name, CycleType cycle, float start, float stop, string node, params RotationKey[] keys)
                => AddSequenceData(name, cycle, start, stop, node, RotationData(keys));

            private static NiTransformData EmptyData() => new()
            {
                Translations = new KeyGroup<Vector3> { Interpolation = KeyType.LINEAR_KEY, NumKeys = 0, Keys = new List<Key<Vector3>>() },
                Scales = new KeyGroup<float> { Interpolation = KeyType.LINEAR_KEY, NumKeys = 0, Keys = new List<Key<float>>() },
                NumRotationKeys = 0,
            };

            private static NiTransformData TranslationData(VectorKey[] keys)
            {
                var data = EmptyData();
                data.Translations = new KeyGroup<Vector3>
                {
                    Interpolation = KeyType.LINEAR_KEY,
                    NumKeys = (uint)keys.Length,
                    Keys = keys.Select(k => new Key<Vector3> { Time = k.Time, Value = k.Value }).ToList(),
                };
                return data;
            }

            private static NiTransformData RotationData(params RotationKey[] keys)
            {
                var data = EmptyData();
                data.NumRotationKeys = (uint)keys.Length;
                data.RotationType = KeyType.LINEAR_KEY;
                data.QuaternionKeys = keys.Select(k => new QuatKey<Quaternion> { Time = k.Time, Value = k.Value }).ToList();
                return data;
            }

            private void AddSequenceData(string name, CycleType cycle, float start, float stop, string node, NiTransformData data)
            {
                var interpolator = new NiTransformInterpolator
                {
                    Transform = new NiQuatTransform { Translation = new Vector3(UnsetValue), Rotation = new Quaternion(UnsetValue, UnsetValue, UnsetValue, UnsetValue), Scale = UnsetValue },
                    Data = new NiBlockRef<NiTransformData>(Nif.AddBlock(data)),
                };
                var sequence = new NiControllerSequence
                {
                    Name = new NiStringRef(name),
                    CycleType = cycle,
                    StartTime = start,
                    StopTime = stop,
                    Frequency = 1f,
                    TextKeys_NiBlockRef_NTKED = new NiBlockRef<NiTextKeyExtraData>(-1),
                    ControlledBlocks = new List<ControlledBlock>
                    {
                        new()
                        {
                            NodeName = new NiStringRef(node),
                            Controller = new NiBlockRef<NiTimeController>(-1),
                            Interpolator = new NiBlockRef<NiInterpolator>(Nif.AddBlock(interpolator)),
                        },
                    },
                };
                int sequenceIndex = Nif.AddBlock(sequence);
                var manager = Nif.Blocks.OfType<NiControllerManager>().FirstOrDefault();
                if (manager == null)
                {
                    manager = new NiControllerManager { ControllerSequences = new NiBlockRefArray<NiControllerSequence>() };
                    Nif.AddBlock(manager);
                }
                manager.ControllerSequences.AddBlockRef(sequenceIndex);
            }
        }
    }
}
