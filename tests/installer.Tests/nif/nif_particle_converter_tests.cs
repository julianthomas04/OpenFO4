using System.Numerics;
using System.Reflection;
using System.Text.Json.Nodes;
using AssetConverter;
using NiflySharp;
using NiflySharp.Blocks;
using NiflySharp.Enums;
using NiflySharp.Structs;
using Xunit;

namespace InstallerTests.Nif
{
    [Collection(StaticStateCollections.NifConverterState)]
    public class NifParticleConverterTests
    {
        private const float Tolerance = 1e-4f;
        private const float UnsetValue = -3.4028235E+38f;

        // Shaped like Effects\MPSFireMed01: a master with two cylinder-emitter systems whose
        // emitter controller carries a constant birth rate of 30 and an on/off timeline, atlas of 2x2.
        private const string FireSystemGolden = """
        {
          "v": 1, "world_space": true, "max_particles": 240, "material": 3,
          "fade": { "near_begin": 10.0, "near_end": 5.0, "far_begin": 0, "far_end": 0 },
          "data": {
            "aspect_ratio": 1.2, "aspect_flags": 257, "speed_to_aspect": [0.5, 0.3, 9.75],
            "subtexture_rects": [[0, 0.5, 0, 0.5], [0.5, 0.5, 0, 0.5], [0, 0.5, 0.5, 0.5], [0.5, 0.5, 0.5, 0.5]]
          },
          "emitters": [{
            "name": "NiPSysCylinderEmitter:0", "type": "cylinder", "node": "Emitter01",
            "radius": 2.25, "height": 0.8,
            "speed": 9.0, "speed_variation": 9.0, "declination": 0.0, "declination_variation": 0.50265485,
            "planar_angle": 0.0, "planar_angle_variation": 1.0053097, "initial_color": [1, 1, 1, 1],
            "initial_radius": 0.5, "radius_variation": 0.35, "life_span": 1.3333334, "life_span_variation": 1.3333334,
            "emitter_ctlr": {
              "birth_rate": { "constant": 30.0 },
              "active": { "times": [0.0, 3.3333333], "values": [1, 0] },
              "cycle": "loop", "start": 0.0, "stop": 3.3333333, "frequency": 1.0, "phase": 14.731284
            }
          }],
          "modifiers": [
            { "name": "LOD", "order": 1, "active": true, "type": "lod",
              "begin": 0.1, "end": 0.7, "end_emit_scale": 0.2, "end_size": 1.0 },
            { "name": "Color", "order": 3000, "active": true, "type": "simple_color",
              "fade_in_percent": 0.2, "fade_out_percent": 0.4, "color1_end_percent": 0.0,
              "color2_start_percent": 0.55, "color2_end_percent": 0.56, "color3_start_percent": 1.0,
              "colors": [[1, 1, 1, 0], [1, 0.5, 0.25, 1], [0, 0, 0, 0]] },
            { "name": "SubTex", "order": 3000, "active": true, "type": "sub_tex",
              "start_frame": 0, "start_frame_fudge": 4, "end_frame": 3, "loop_start_frame": 0,
              "loop_start_frame_fudge": 0, "frame_count": 60, "frame_count_fudge": 10 },
            { "name": "Scale", "order": 3000, "active": true, "type": "scale", "scales": [1.0, 2.0, 0.5] },
            { "name": "Gravity", "order": 4000, "active": true, "type": "gravity", "object": "GravityObj",
              "axis": [0, 1, 0], "decay": 0, "strength": 13.5, "force": "planar", "turbulence": 0.08,
              "turbulence_scale": 15, "world_aligned": true },
            { "name": "DragZ", "order": 4000, "active": true, "type": "drag", "object": "GravityObj",
              "axis": [0, 1, 0], "percentage": 0.05, "range": null, "range_falloff": 0,
              "active_track": { "cycle": "loop", "start": 0.0, "stop": 3.3333333, "frequency": 1.0, "phase": 0.0,
                                "times": [0.0, 3.3333333], "values": [1, 0] } },
            { "name": "Wind", "order": 4000, "active": true, "type": "wind", "strength": 1.5 }
          ],
          "tracks": [{
            "target": "NiPSysCylinderEmitter:0", "field": "speed",
            "track": { "cycle": "loop", "start": 0.0, "stop": 2.0, "frequency": 1.0, "phase": 0.0,
                       "times": [0.0, 2.0], "values": [9.0, 12.0] }
          }],
          "update": { "frequency": 1.0, "phase": 0.0 }
        }
        """;

        [Fact]
        public void Build_MpsFireMedShapedSystem_MatchesTheGoldenExtras()
        {
            var scene = new ParticleScene();
            int system = scene.AddFireSystem("Fire01");

            var result = ParticleSystemExtras.Build(scene.Nif, system, 3);

            Assert.NotNull(result);
            AssertJsonEquivalent(JsonNode.Parse(FireSystemGolden)!, result!.Extras);
            Assert.Equal(new[] { "NiPSysBombModifier" }, result.SkippedModifiers);
        }

        [Fact]
        public void Build_MasterParticleSystem_ListsTheSystemNodeNames()
        {
            var scene = new ParticleScene();
            int first = scene.AddFireSystem("Fire01");
            int second = scene.AddFireSystem("Smoke01");
            int master = scene.Nif.AddBlock(new BSMasterParticleSystem { MaxEmitterObjects = 20, ParticleSystems = RefArray<NiAVObject>(first, second) });

            var json = ParticleSystemExtras.BuildMaster(scene.Nif, master);

            AssertJsonEquivalent(JsonNode.Parse("""{ "v": 1, "max_emitter_objects": 20, "systems": ["Fire01", "Smoke01"] }""")!, json);
        }

        [Fact]
        public void Build_UnsupportedModifier_IsSkippedReportedAndCountedWhileTheSystemStays()
        {
            var scene = new ParticleScene();
            int system = scene.AddFireSystem("Fire01", extraModifiers: new NiPSysModifier[]
            {
                new NiPSysBombModifier { Name = new NiStringRef("Bomb"), Order = NiPSysModifierOrder.ORDER_FORCE },
                new BSParentVelocityModifier { Name = new NiStringRef("Parent") },
            });

            var result = ParticleSystemExtras.Build(scene.Nif, system, 0)!;

            Assert.Equal(new[] { "NiPSysBombModifier", "NiPSysBombModifier", "BSParentVelocityModifier" }, result.SkippedModifiers);
            Assert.Single((JsonArray)result.Extras["emitters"]!);
            Assert.DoesNotContain(((JsonArray)result.Extras["modifiers"]!).Select(m => (string)m!["name"]!), n => n is "Bomb" or "Parent");
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(NifAnimationStats.UnsupportedPrefix + "particle modifier BSParentVelocityModifier: ", StringComparison.Ordinal));
        }

        [Fact]
        public void Build_ManagerControlledEmitter_TakesRateAndActivityFromTheLoopingSequence()
        {
            var scene = new ParticleScene();
            int system = scene.AddSystem("Spray", new NiPSysBoxEmitter { Name = new NiStringRef("Box"), Width = 20f, Height = 40f, Depth = 60f });
            int controller = scene.AddEmitterController(system, "Box", scene.Nif.AddBlock(new NiBlendFloatInterpolator { Value = UnsetValue }), scene.Nif.AddBlock(new NiBlendBoolInterpolator()));
            scene.AddSequence("autoPlay", CycleType.CYCLE_CLAMP, 0f, 1f / 30f, (controller, scene.FloatInterpolator(KeyType.LINEAR_KEY, (0f, 0f), (1f, 0f))));
            scene.AddSequence("autoLoop", CycleType.CYCLE_LOOP, 0f, 50f,
                (controller, scene.FloatInterpolator(KeyType.LINEAR_KEY, (0f, 6f), (50f, 6f))),
                (controller, scene.BoolInterpolator((0f, 1), (50f, 0))));

            var emitter = (JsonObject)((JsonArray)ParticleSystemExtras.Build(scene.Nif, system, 0)!.Extras["emitters"]!)[0]!;

            AssertJsonEquivalent(JsonNode.Parse("""
                { "birth_rate": { "times": [0], "values": [6] }, "active": { "times": [0, 50], "values": [1, 0] },
                  "cycle": "loop", "start": 0, "stop": 50, "frequency": 1, "phase": 0 }
                """)!, emitter["emitter_ctlr"]!);
            Assert.Equal(("box", 2f, 6f, 4f), ((string)emitter["type"]!, (float)emitter["width"]!, (float)emitter["height"]!, (float)emitter["depth"]!));
        }

        [Fact]
        public void Build_SphereAndMeshEmitters_CarryTheirVolumeAndMeshNodes()
        {
            var scene = new ParticleScene();
            int mesh = scene.Nif.AddBlock(new NiNode { Name = new NiStringRef("EmitSource") });
            var meshEmitter = new NiPSysMeshEmitter
            {
                Name = new NiStringRef("MeshEmitter"),
                EmitterMeshes = PtrArray<NiAVObject>(mesh),
                EmissionType = EmitFrom.EMIT_FROM_FACE_CENTER,
                InitialVelocityType = VelocityType.VELOCITY_USE_NORMALS,
            };
            var sphereEmitter = new NiPSysSphereEmitter { Name = new NiStringRef("Sphere"), Radius = 50f, InitialRadius = -12f };

            var sphere = (JsonObject)((JsonArray)ParticleSystemExtras.Build(scene.Nif, scene.AddSystem("A", sphereEmitter), 0)!.Extras["emitters"]!)[0]!;
            var meshJson = (JsonObject)((JsonArray)ParticleSystemExtras.Build(scene.Nif, scene.AddSystem("B", meshEmitter), 0)!.Extras["emitters"]!)[0]!;

            Assert.Equal(("sphere", 5f, 0f), ((string)sphere["type"]!, (float)sphere["radius"]!, (float)sphere["initial_radius"]!));
            Assert.Equal("mesh", (string)meshJson["type"]!);
            Assert.Equal(new[] { "EmitSource" }, ((JsonArray)meshJson["mesh_nodes"]!).Select(n => (string)n!));
            Assert.Equal("face_centre", (string)meshJson["emission_type"]!);
        }

        [Fact]
        public void Build_MeshEmitterEmittingFromEdges_SkipsTheSystemAndCountsIt()
        {
            var scene = new ParticleScene();
            var emitter = new NiPSysMeshEmitter { Name = new NiStringRef("MeshEmitter"), EmitterMeshes = PtrArray<NiAVObject>(), EmissionType = EmitFrom.EMIT_FROM_EDGE_SURFACE };

            var result = ParticleSystemExtras.Build(scene.Nif, scene.AddSystem("A", emitter), 0);

            Assert.Null(result);
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(NifAnimationStats.UnsupportedPrefix + "particle system (emitter MeshEmitter has an unsupported emission type): ", StringComparison.Ordinal));
        }

        [Fact]
        public void Build_MeshEmitterEmittingFromVertices_WritesTheVerticesEmissionType()
        {
            var scene = new ParticleScene();
            int source = scene.Nif.AddBlock(new NiNode { Name = new NiStringRef("emitFireBall:0") });
            var emitter = new NiPSysMeshEmitter { Name = new NiStringRef("MeshEmitter"), EmitterMeshes = PtrArray<NiAVObject>(source), EmissionType = EmitFrom.EMIT_FROM_VERTICES };

            var json = ParticleSystemExtras.Build(scene.Nif, scene.AddSystem("A", emitter), 0)!.Extras["emitters"]![0]!;

            Assert.Equal("vertices", (string)json["emission_type"]!);
            Assert.Equal(new[] { "emitFireBall:0" }, ((JsonArray)json["mesh_nodes"]!).Select(n => (string)n!));
        }

        [Fact]
        public void Build_MeshEmitterWithoutEmitterObject_NamesTheSystemNodeAndCountsIt()
        {
            var scene = new ParticleScene();
            var emitter = new NiPSysMeshEmitter { Name = new NiStringRef("MeshEmitter"), EmitterMeshes = PtrArray<NiAVObject>(), EmissionType = EmitFrom.EMIT_FROM_FACE_SURFACE };

            var json = ParticleSystemExtras.Build(scene.Nif, scene.AddSystem("FlameSystem", emitter), 0)!.Extras["emitters"]![0]!;

            Assert.Equal("FlameSystem", (string)json["node"]!);
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(ParticleSystemExtras.MissingObjectCounter, StringComparison.Ordinal));
        }

        [Fact]
        public void Build_ForceModifiersWithoutObject_NameTheSystemNode()
        {
            var scene = new ParticleScene();
            int system = scene.AddSystem("FlameSystem", new NiPSysSphereEmitter { Name = new NiStringRef("Sphere") }, modifiers: new NiPSysModifier[]
            {
                new NiPSysDragModifier { Name = new NiStringRef("Drag"), DragObject = new NiBlockPtr<NiAVObject>(-1), DragAxis = Vector3.UnitX, Percentage = 1f, Range = 1f },
                new NiPSysGravityModifier { Name = new NiStringRef("Gravity"), GravityObject = new NiBlockPtr<NiAVObject>(-1), ForceType = ForceType.FORCE_PLANAR, GravityAxis = Vector3.UnitZ },
                new BSPSysInheritVelocityModifier { Name = new NiStringRef("Inherit"), InheritObject = new NiBlockPtr<NiAVObject>(-1) },
            });

            var modifiers = (JsonArray)ParticleSystemExtras.Build(scene.Nif, system, 0)!.Extras["modifiers"]!;

            Assert.Equal(new[] { "FlameSystem", "FlameSystem", "FlameSystem" }, modifiers.Select(m => (string)m!["object"]!));
        }

        [Fact]
        public void Build_UnnamedEmitterObject_UsesTheGeneratedNodeName()
        {
            var scene = new ParticleScene();
            int unnamed = scene.Nif.AddBlock(new NiNode());
            var emitter = new NiPSysSphereEmitter { Name = new NiStringRef("Sphere"), EmitterObject = new NiBlockPtr<NiNode>(unnamed) };

            var json = ParticleSystemExtras.Build(scene.Nif, scene.AddSystem("A", emitter), 0)!.Extras["emitters"]![0]!;

            Assert.Equal($"Node_{unnamed}", (string)json["node"]!);
        }

        [Fact]
        public void Build_EmitterWithNonFiniteValue_LeavesTheSystemOutAndCountsIt()
        {
            var scene = new ParticleScene();
            int system = scene.AddSystem("A", new NiPSysSphereEmitter { Name = new NiStringRef("Sphere"), Speed = float.NaN });

            Assert.Null(ParticleSystemExtras.Build(scene.Nif, system, 0));
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(ParticleSystemExtras.NonFiniteSystemCounter, StringComparison.Ordinal));
        }

        [Fact]
        public void Build_ControllerWithNonFiniteFrequency_IsDroppedAndTheSystemStays()
        {
            var scene = new ParticleScene();
            int system = scene.AddSystem("A", new NiPSysSphereEmitter { Name = new NiStringRef("Sphere") });
            scene.AddController<NiPSysEmitterSpeedCtlr>(system, c =>
            {
                c.ModifierName = new NiStringRef("Sphere");
                c.Interpolator = new NiBlockRef<NiInterpolator>(scene.FloatInterpolator(KeyType.LINEAR_KEY, (0f, 1f), (1f, 2f)));
                c.Frequency = float.PositiveInfinity;
            });

            var extras = ParticleSystemExtras.Build(scene.Nif, system, 0)!.Extras;

            Assert.Empty((JsonArray)extras["tracks"]!);
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(ParticleSystemExtras.NonFiniteControllerCounter, StringComparison.Ordinal));
        }

        [Theory]
        [InlineData(0f, 0f)]
        [InlineData(5f, 1f)]
        [InlineData(0f, 0.5f)]
        public void Build_ControllerRangeThatDoesNotCoverItsKeys_UsesTheKeyRange(float start, float stop)
        {
            var scene = new ParticleScene();
            int system = scene.AddSystem("A", new NiPSysSphereEmitter { Name = new NiStringRef("Sphere") });
            scene.AddController<NiPSysEmitterSpeedCtlr>(system, c =>
            {
                c.ModifierName = new NiStringRef("Sphere");
                c.Interpolator = new NiBlockRef<NiInterpolator>(scene.FloatInterpolator(KeyType.LINEAR_KEY, (0f, 1f), (2f, 2f)));
                c.StartTime = start;
                c.StopTime = stop;
            });

            var track = ParticleSystemExtras.Build(scene.Nif, system, 0)!.Extras["tracks"]![0]!["track"]!;

            Assert.Equal((0.0, 2.0), (Number(track["start"]!), Number(track["stop"]!)));
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(ClipTiming.RangeReplacedCounter, StringComparison.Ordinal));
        }

        [Fact]
        public void Build_EmitterWithoutController_NeverEmitsAndIsCounted()
        {
            var scene = new ParticleScene();
            int system = scene.AddSystem("A", new NiPSysSphereEmitter { Name = new NiStringRef("Sphere") });

            var ctlr = ParticleSystemExtras.Build(scene.Nif, system, 0)!.Extras["emitters"]![0]!["emitter_ctlr"]!;

            Assert.Equal(0.0, Number(ctlr["birth_rate"]!["constant"]!));
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(ParticleSystemExtras.EmitterWithoutControllerCounter, StringComparison.Ordinal));
        }

        [Fact]
        public void Build_SystemWithoutEmitter_IsSkipped()
        {
            var scene = new ParticleScene();
            int system = scene.AddSystem("A", null);

            Assert.Null(ParticleSystemExtras.Build(scene.Nif, system, 0));
        }

        [Fact]
        public void Build_DragWithBoundedRangeAndNoAtlas_ConvertsTheRangeAndEmitsEmptyRects()
        {
            var scene = new ParticleScene();
            int system = scene.AddSystem("A", new NiPSysSphereEmitter { Name = new NiStringRef("Sphere") },
                modifiers: new NiPSysModifier[] { new NiPSysDragModifier { Name = new NiStringRef("Drag"), DragObject = new NiBlockPtr<NiAVObject>(-1), DragAxis = Vector3.UnitX, Percentage = 1f, Range = 120f, RangeFalloff = 2f } },
                hasAtlas: false);

            var extras = ParticleSystemExtras.Build(scene.Nif, system, 0)!.Extras;

            Assert.Empty((JsonArray)extras["data"]!["subtexture_rects"]!);
            var drag = extras["modifiers"]![0]!;
            Assert.Equal(12f, (float)drag["range"]!, Tolerance);
            Assert.Equal(new[] { 1f, 0f, 0f }, ((JsonArray)drag["axis"]!).Select(v => (float)v!));
        }

        [Fact]
        public void Build_FieldControllerBecomesATrackAndUnsupportedControllerIsCounted()
        {
            var scene = new ParticleScene();
            int system = scene.AddSystem("A", new NiPSysSphereEmitter { Name = new NiStringRef("Sphere") });
            scene.AddFloatController<NiPSysEmitterDeclinationCtlr>(system, "Sphere", scene.FloatInterpolator(KeyType.LINEAR_KEY, (0f, 0f), (1f, 1f)));
            scene.AddController<NiTransformController>(system, c => { });

            var extras = ParticleSystemExtras.Build(scene.Nif, system, 0)!.Extras;

            Assert.Equal("declination", (string)extras["tracks"]![0]!["field"]!);
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(NifAnimationStats.UnsupportedPrefix + "particle controller NiTransformController: ", StringComparison.Ordinal));
        }

        [Fact]
        public void Build_FieldControllerWithNonFiniteKey_IsDroppedAndCountedInsteadOfFailingTheFile()
        {
            var scene = new ParticleScene();
            int system = scene.AddSystem("A", new NiPSysSphereEmitter { Name = new NiStringRef("Sphere") });
            scene.AddFloatController<NiPSysEmitterSpeedCtlr>(system, "Sphere", scene.FloatInterpolator(KeyType.LINEAR_KEY, (0f, 1f), (1f, float.NaN)));

            var extras = ParticleSystemExtras.Build(scene.Nif, system, 0)!.Extras;

            Assert.Empty((JsonArray)extras["tracks"]!);
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(ControllerSources.NonFiniteKeysCounter, StringComparison.Ordinal));
        }

        [Theory]
        [InlineData("Shared")]
        [InlineData("")]
        public void Build_EmittersAndModifiersSharingAName_EachCarryTheirOwnControllerNodes(string name)
        {
            var scene = new ParticleScene();
            int system = scene.AddSystem("A", new NiPSysSphereEmitter { Name = new NiStringRef(name) }, modifiers: new NiPSysModifier[]
            {
                new NiPSysBoxEmitter { Name = new NiStringRef(name), Width = 10f, Height = 10f, Depth = 10f },
                new NiPSysDragModifier { Name = new NiStringRef(name), DragObject = new NiBlockPtr<NiAVObject>(-1), DragAxis = Vector3.UnitX, Percentage = 1f, Range = float.MaxValue },
                new NiPSysDragModifier { Name = new NiStringRef(name), DragObject = new NiBlockPtr<NiAVObject>(-1), DragAxis = Vector3.UnitY, Percentage = 1f, Range = float.MaxValue },
            });
            scene.AddEmitterController(system, name, scene.ConstantFloatInterpolator(5f), scene.BoolInterpolator((0f, 1), (1f, 0)));
            scene.AddController<NiPSysModifierActiveCtlr>(system, c =>
            {
                c.ModifierName = new NiStringRef(name);
                c.Interpolator = new NiBlockRef<NiInterpolator>(scene.BoolInterpolator((0f, 1), (1f, 0)));
            });

            var extras = ParticleSystemExtras.Build(scene.Nif, system, 0)!.Extras;

            var emitters = ((JsonArray)extras["emitters"]!).Select(e => e!["emitter_ctlr"]!).ToList();
            var tracks = ((JsonArray)extras["modifiers"]!).Select(m => m!["active_track"]).Where(t => t != null).ToList();
            Assert.Equal(2, emitters.Count);
            Assert.Equal(2, tracks.Count);
            Assert.All(emitters, e => Assert.Equal(5.0, Number(e["birth_rate"]!["constant"]!)));
            Assert.All(tracks, t => Assert.Equal(2, ((JsonArray)t!["times"]!).Count));
        }

        [Fact]
        public void Build_SimpleColorModifier_MapsFileOrderedPercentsOntoTheEngineColourRamp()
        {
            var scene = new ParticleScene();
            int system = scene.AddSystem("A", new NiPSysSphereEmitter { Name = new NiStringRef("Sphere") }, modifiers: new NiPSysModifier[]
            {
                new BSPSysSimpleColorModifier
                {
                    Name = new NiStringRef("Color"), FadeInPercent = 0.1f, FadeOutPercent = 0.9f,
                    Color1EndPercent = 0.2f, Color1StartPercent = 0.3f, Color2EndPercent = 0.6f, Color2StartPercent = 0.8f,
                    Colors = new[] { new Color4(), new Color4(), new Color4() },
                },
            });

            var color = ParticleSystemExtras.Build(scene.Nif, system, 0)!.Extras["modifiers"]!.AsArray().Single(m => (string)m!["type"]! == "simple_color")!;

            Assert.Equal(
                new[] { 0.2f, 0.3f, 0.6f, 0.8f },
                new[] { "color1_end_percent", "color2_start_percent", "color2_end_percent", "color3_start_percent" }.Select(k => (float)color[k]!));
        }

        [Fact]
        public void MaxVerticesField_StillExistsOnTheNiflySharpType()
        {
            var field = typeof(NiGeometryData).GetField("_bSMaxVertices", BindingFlags.NonPublic | BindingFlags.Instance);

            Assert.NotNull(field);
            Assert.Equal(typeof(ushort), field!.FieldType);
        }

        [Fact]
        public void Build_BlockThatIsNotAParticleSystem_Throws()
        {
            var scene = new ParticleScene();
            int node = scene.Nif.AddBlock(new NiNode());

            Assert.Throws<NifAnimationException>(() => ParticleSystemExtras.Build(scene.Nif, node, 0));
            Assert.Throws<NifAnimationException>(() => ParticleSystemExtras.BuildMaster(scene.Nif, node));
        }

        private static void AssertJsonEquivalent(JsonNode expected, JsonNode actual, string path = "$")
        {
            switch (expected)
            {
                case JsonObject expectedObject:
                    var actualObject = Assert.IsType<JsonObject>(actual);
                    Assert.True(expectedObject.Select(p => p.Key).OrderBy(k => k, StringComparer.Ordinal).SequenceEqual(actualObject.Select(p => p.Key).OrderBy(k => k, StringComparer.Ordinal)),
                        $"{path}: keys [{string.Join(",", expectedObject.Select(p => p.Key))}] vs [{string.Join(",", actualObject.Select(p => p.Key))}]");
                    foreach (var (key, value) in expectedObject)
                        AssertJsonEquivalent(value!, actualObject[key]!, $"{path}.{key}");
                    break;
                case JsonArray expectedArray:
                    var actualArray = Assert.IsType<JsonArray>(actual);
                    Assert.True(expectedArray.Count == actualArray.Count, $"{path}: length {expectedArray.Count} vs {actualArray.Count}");
                    for (int i = 0; i < expectedArray.Count; i++)
                        AssertJsonEquivalent(expectedArray[i]!, actualArray[i]!, $"{path}[{i}]");
                    break;
                case null:
                    Assert.Null(actual);
                    break;
                default:
                    var expectedValue = expected.AsValue();
                    if (expectedValue.TryGetValue<bool>(out var flag))
                        Assert.True(flag == actual.GetValue<bool>(), path);
                    else if (expectedValue.TryGetValue<string>(out var text))
                        Assert.True(text == actual.GetValue<string>(), $"{path}: '{text}' vs '{actual.GetValue<string>()}'");
                    else
                    {
                        double want = Number(expected), got = Number(actual);
                        Assert.True(Math.Abs(want - got) <= 1e-4 * Math.Max(1.0, Math.Abs(want)), $"{path}: {want} vs {got}");
                    }
                    break;
            }
        }

        private static double Number(JsonNode node) => double.Parse(node.ToJsonString(), System.Globalization.CultureInfo.InvariantCulture);

        private static NiBlockRefArray<T> RefArray<T>(params int[] indices) where T : NiObject
        {
            var array = new NiBlockRefArray<T>();
            foreach (int index in indices)
                array.AddBlockRef(index);
            return array;
        }

        private static NiBlockPtrArray<T> PtrArray<T>(params int[] indices) where T : NiObject
        {
            var array = new NiBlockPtrArray<T>();
            foreach (int index in indices)
                array.AddBlockRef(index);
            return array;
        }

        private sealed class ParticleScene
        {
            private static readonly FieldInfo MaxVertices = typeof(NiGeometryData).GetField("_bSMaxVertices", BindingFlags.NonPublic | BindingFlags.Instance)!;

            public NifFile Nif { get; } = new();

            public ParticleScene()
            {
                Nif.Create(NiVersion.GetFO4(), true);
            }

            // The fire system of the golden test: every supported modifier type once.
            public int AddFireSystem(string name, IEnumerable<NiPSysModifier>? extraModifiers = null)
            {
                int emitterNode = Nif.AddBlock(new NiNode { Name = new NiStringRef("Emitter01") });
                int gravityNode = Nif.AddBlock(new NiNode { Name = new NiStringRef("GravityObj") });
                var emitter = new NiPSysCylinderEmitter
                {
                    Name = new NiStringRef("NiPSysCylinderEmitter:0"),
                    Order = NiPSysModifierOrder.ORDER_EMITTER,
                    EmitterObject = new NiBlockPtr<NiNode>(emitterNode),
                    Radius = 22.5f,
                    Height = 8f,
                    Speed = 90f,
                    SpeedVariation = 90f,
                    Declination = 0f,
                    DeclinationVariation = 0.50265485f,
                    PlanarAngle = 0f,
                    PlanarAngleVariation = 1.0053097f,
                    InitialColor = new Color4 { R = 1, G = 1, B = 1, A = 1 },
                    InitialRadius = 5f,
                    RadiusVariation = 3.5f,
                    LifeSpan = 1.3333334f,
                    LifeSpanVariation = 1.3333334f,
                };
                var modifiers = new List<NiPSysModifier>
                {
                    new NiPSysAgeDeathModifier { Name = new NiStringRef("AgeDeath") },
                    new BSPSysLODModifier { Name = new NiStringRef("LOD"), Order = NiPSysModifierOrder.ORDER_BSLOD, LODBeginDistance = 0.1f, LODEndDistance = 0.7f, EndEmitScale = 0.2f, EndSize = 1f },
                    new NiPSysSpawnModifier { Name = new NiStringRef("Spawn") },
                    new BSPSysSimpleColorModifier
                    {
                        Name = new NiStringRef("Color"), Order = NiPSysModifierOrder.ORDER_GENERAL,
                        FadeInPercent = 0.2f, FadeOutPercent = 0.4f, Color1EndPercent = 0f, Color1StartPercent = 0.55f, Color2EndPercent = 0.56f, Color2StartPercent = 1f,
                        Colors = new[] { new Color4 { R = 1, G = 1, B = 1, A = 0 }, new Color4 { R = 1, G = 0.5f, B = 0.25f, A = 1 }, new Color4() },
                    },
                    new BSPSysSubTexModifier
                    {
                        Name = new NiStringRef("SubTex"), Order = NiPSysModifierOrder.ORDER_GENERAL,
                        StartFrame = 0f, StartFrameFudge = 4f, EndFrame = 3f, LoopStartFrame = 0f, LoopStartFrameFudge = 0f, FrameCount = 60f, FrameCountFudge = 10f,
                    },
                    new BSPSysScaleModifier { Name = new NiStringRef("Scale"), Order = NiPSysModifierOrder.ORDER_GENERAL, NumScales = 3, Scales = new List<float> { 1f, 2f, 0.5f } },
                    new NiPSysGravityModifier
                    {
                        Name = new NiStringRef("Gravity"), Order = NiPSysModifierOrder.ORDER_FORCE,
                        GravityObject = new NiBlockPtr<NiAVObject>(gravityNode), GravityAxis = Vector3.UnitZ, Decay = 0f, Strength = 135f,
                        ForceType = ForceType.FORCE_PLANAR, Turbulence = 0.08f, TurbulenceScale = 15f, WorldAligned = true,
                    },
                    new NiPSysDragModifier
                    {
                        Name = new NiStringRef("DragZ"), Order = NiPSysModifierOrder.ORDER_FORCE,
                        DragObject = new NiBlockPtr<NiAVObject>(gravityNode), DragAxis = Vector3.UnitZ, Percentage = 0.05f, Range = float.MaxValue, RangeFalloff = 0f,
                    },
                    new BSWindModifier { Name = new NiStringRef("Wind"), Order = NiPSysModifierOrder.ORDER_FORCE, Strength = 1.5f },
                    new NiPSysPositionModifier { Name = new NiStringRef("Position") },
                    new NiPSysBoundUpdateModifier { Name = new NiStringRef("Bound") },
                    new NiPSysBombModifier { Name = new NiStringRef("Bomb"), Order = NiPSysModifierOrder.ORDER_FORCE },
                };
                modifiers.InsertRange(2, new NiPSysModifier[] { emitter });
                modifiers.AddRange(extraModifiers ?? Array.Empty<NiPSysModifier>());
                int system = AddSystemBlock(name, modifiers, hasAtlas: true);

                AddEmitterController(system, "NiPSysCylinderEmitter:0", ConstantFloatInterpolator(30f), BoolInterpolator((0f, 1), (3.3333333f, 0)), 14.731284f, 3.3333333f);
                AddController<NiPSysUpdateCtlr>(system, c => { });
                AddController<NiPSysModifierActiveCtlr>(system, c =>
                {
                    c.ModifierName = new NiStringRef("DragZ");
                    c.Interpolator = new NiBlockRef<NiInterpolator>(BoolInterpolator((0f, 1), (3.3333333f, 0)));
                    c.StopTime = 3.3333333f;
                });
                AddFloatController<NiPSysEmitterSpeedCtlr>(system, "NiPSysCylinderEmitter:0", FloatInterpolator(KeyType.LINEAR_KEY, (0f, 90f), (2f, 120f)), 2f);
                var particleSystem = (NiParticleSystem)Nif.Blocks[system];
                particleSystem.NearBegin = 100;
                particleSystem.NearEnd = 50;
                return system;
            }

            public int AddSystem(string name, NiPSysEmitter? emitter, IEnumerable<NiPSysModifier>? modifiers = null, bool hasAtlas = true)
            {
                var all = new List<NiPSysModifier>();
                if (emitter != null)
                    all.Add(emitter);
                all.AddRange(modifiers ?? Array.Empty<NiPSysModifier>());
                return AddSystemBlock(name, all, hasAtlas);
            }

            private int AddSystemBlock(string name, IReadOnlyList<NiPSysModifier> modifiers, bool hasAtlas)
            {
                var data = new NiPSysData
                {
                    AspectRatio = 1.2f,
                    AspectFlags = (AspectFlags)257,
                    SpeedtoAspectAspect2 = 0.5f,
                    SpeedtoAspectSpeed1 = 3f,
                    SpeedtoAspectSpeed2 = 97.5f,
                    HasTextureIndices = hasAtlas,
                    SubtextureOffsets = hasAtlas
                        ? new List<Vector4> { new(0, 0.5f, 0, 0.5f), new(0.5f, 0.5f, 0, 0.5f), new(0, 0.5f, 0.5f, 0.5f), new(0.5f, 0.5f, 0.5f, 0.5f) }
                        : new List<Vector4>(),
                };
                MaxVertices.SetValue(data, (ushort)240);
                var system = new NiParticleSystem
                {
                    Name = new NiStringRef(name),
                    WorldSpace = true,
                    Data_NiBlockRef_NPSD = new NiBlockRef<NiPSysData>(Nif.AddBlock(data)),
                    Controller = new NiBlockRef<NiTimeController>(-1),
                    Modifiers = new NiBlockRefArray<NiPSysModifier>(),
                };
                foreach (var modifier in modifiers)
                    system.Modifiers.AddBlockRef(Nif.AddBlock(modifier));
                return Nif.AddBlock(system);
            }

            public int AddController<T>(int system, Action<T> configure) where T : NiTimeController, new()
            {
                var controller = new T { StartTime = 0f, StopTime = 1f, Frequency = 1f, Flags = new NiflySharp.Bitfields.TimeControllerFlags { CycleType = CycleType.CYCLE_LOOP }, NextController = new NiBlockRef<NiTimeController>(-1) };
                configure(controller);
                int index = Nif.AddBlock(controller);
                var owner = (NiObjectNET)Nif.Blocks[system];
                if (owner.Controller.Index < 0)
                {
                    owner.Controller = new NiBlockRef<NiTimeController>(index);
                    return index;
                }
                var tail = (NiTimeController)Nif.Blocks[owner.Controller.Index];
                while (tail.NextController.Index >= 0)
                    tail = (NiTimeController)Nif.Blocks[tail.NextController.Index];
                tail.NextController = new NiBlockRef<NiTimeController>(index);
                return index;
            }

            public int AddEmitterController(int system, string modifierName, int rateInterpolator, int visibilityInterpolator, float phase = 0f, float stop = 1f)
                => AddController<NiPSysEmitterCtlr>(system, c =>
                {
                    c.ModifierName = new NiStringRef(modifierName);
                    c.Interpolator = new NiBlockRef<NiInterpolator>(rateInterpolator);
                    c.VisibilityInterpolator = new NiBlockRef<NiInterpolator>(visibilityInterpolator);
                    c.Phase = phase;
                    c.StopTime = stop;
                });

            public int AddFloatController<T>(int system, string modifierName, int interpolator, float stop = 1f) where T : NiPSysModifierFloatCtlr, new()
                => AddController<T>(system, c =>
                {
                    c.ModifierName = new NiStringRef(modifierName);
                    c.Interpolator = new NiBlockRef<NiInterpolator>(interpolator);
                    c.StopTime = stop;
                });

            public int ConstantFloatInterpolator(float value) => Nif.AddBlock(new NiFloatInterpolator { Value = value, Data = new NiBlockRef<NiFloatData>(-1) });

            public int FloatInterpolator(KeyType type, params (float Time, float Value)[] keys)
            {
                var data = new NiFloatData
                {
                    Data = new KeyGroup<float>
                    {
                        Interpolation = type,
                        NumKeys = (uint)keys.Length,
                        Keys = keys.Select(k => new Key<float> { Time = k.Time, Value = k.Value }).ToList(),
                    },
                };
                return Nif.AddBlock(new NiFloatInterpolator { Value = UnsetValue, Data = new NiBlockRef<NiFloatData>(Nif.AddBlock(data)) });
            }

            public int BoolInterpolator(params (float Time, byte Value)[] keys)
            {
                var data = new NiBoolData
                {
                    Data = new KeyGroup<byte>
                    {
                        Interpolation = KeyType.CONST_KEY,
                        NumKeys = (uint)keys.Length,
                        Keys = keys.Select(k => new Key<byte> { Time = k.Time, Value = k.Value }).ToList(),
                    },
                };
                return Nif.AddBlock(new NiBoolInterpolator { Data = new NiBlockRef<NiBoolData>(Nif.AddBlock(data)) });
            }

            public int AddSequence(string name, CycleType cycle, float start, float stop, params (int Controller, int Interpolator)[] blocks)
                => Nif.AddBlock(new NiControllerSequence
                {
                    Name = new NiStringRef(name),
                    CycleType = cycle,
                    StartTime = start,
                    StopTime = stop,
                    Frequency = 1f,
                    ControlledBlocks = blocks.Select(b => new ControlledBlock
                    {
                        Controller = new NiBlockRef<NiTimeController>(b.Controller),
                        Interpolator = new NiBlockRef<NiInterpolator>(b.Interpolator),
                    }).ToList(),
                });
        }
    }
}
