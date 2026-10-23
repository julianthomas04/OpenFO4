using System.Numerics;
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
    public class NifShaderControllerTests
    {
        private const float Tolerance = 1e-4f;
        private const float UnsetValue = -3.4028235E+38f;

        [Fact]
        public void ForShader_BarrelFlameEmissiveMultiple_ManagerControlledTakesTheLoopingSequenceWith157Keys()
        {
            var scene = new ControllerScene();
            int shader = scene.AddEffectShader();
            int controller = scene.AddEffectFloatController(shader, EffectShaderControlledVariable.EmissiveMultiple);
            scene.AddSequence("autoPlay", CycleType.CYCLE_CLAMP, 0f, 1f / 30f, (controller, scene.AddFloatInterpolator(KeyType.QUADRATIC_KEY, new(0f, 0f), new(1f / 30f, 0f))));
            var flicker = Enumerable.Range(0, 157).Select(i => new ScalarKey(i * 0.32f, 2.6f + 0.3f * MathF.Sin(i * 0.4f))).ToArray();
            scene.AddSequence("autoLoop", CycleType.CYCLE_LOOP, 0f, 50f, (controller, scene.AddFloatInterpolator(KeyType.LINEAR_KEY, flicker)));

            var result = ShaderControllerTracks.ForShader(scene.Nif, shader);

            var track = Assert.Single(result.Tracks);
            Assert.Equal("emissive_mult", track.Target);
            Assert.Equal("loop", track.Cycle);
            Assert.Equal(0f, track.Start);
            Assert.Equal(50f, track.Stop);
            Assert.Equal(157, track.Times.Count);
            Assert.Equal(flicker[100].Time, track.Times[100], Tolerance);
            Assert.Equal(flicker[100].Value, Assert.Single(track.Values[100]), Tolerance);
            Assert.Empty(result.Unsupported);
        }

        [Fact]
        public void ForLight_QuadraticDimmer_IsResampledAt30HzThroughTheHermiteCurve()
        {
            var scene = new ControllerScene();
            int light = scene.AddPointLight();
            scene.AddController<NiLightDimmerController>(light, scene.AddFloatInterpolator(KeyType.QUADRATIC_KEY, new(0f, 0f), new(1f, 10f)));

            var result = ShaderControllerTracks.ForLight(scene.Nif, light);

            var track = Assert.Single(result.Tracks);
            Assert.Equal("dimmer", track.Target);
            Assert.Equal(31, track.Times.Count);
            Assert.Equal(1f / 30f, track.Times[1] - track.Times[0], Tolerance);
            Assert.Equal(0f, track.Values[0][0], Tolerance);
            // Zero tangents make the segment a smoothstep: 10 * (3u^2 - 2u^3).
            Assert.Equal(10f * (3f * 0.2f * 0.2f - 2f * 0.2f * 0.2f * 0.2f), ValueAt(track, 0.2f), Tolerance);
            Assert.Equal(5f, ValueAt(track, 0.5f), Tolerance);
            Assert.Equal(10f, track.Values[^1][0], Tolerance);
        }

        [Theory]
        [InlineData(EffectShaderControlledVariable.EmissiveMultiple, "emissive_mult")]
        [InlineData(EffectShaderControlledVariable.AlphaTransparency, "alpha")]
        [InlineData(EffectShaderControlledVariable.UOffset, "u_offset")]
        [InlineData(EffectShaderControlledVariable.UScale, "u_scale")]
        [InlineData(EffectShaderControlledVariable.VOffset, "v_offset")]
        [InlineData(EffectShaderControlledVariable.VScale, "v_scale")]
        public void ForShader_EffectFloatVariable_MapsToContractTarget(EffectShaderControlledVariable variable, string expected)
        {
            var scene = new ControllerScene();
            int shader = scene.AddEffectShader();
            scene.AddEffectFloatController(shader, variable, scene.AddFloatInterpolator(KeyType.LINEAR_KEY, new(0f, 1f), new(1f, 2f)));

            var track = Assert.Single(ShaderControllerTracks.ForShader(scene.Nif, shader).Tracks);

            Assert.Equal(expected, track.Target);
        }

        [Theory]
        [InlineData(LightingShaderControlledFloat.RefractionStrength, "refraction_strength")]
        [InlineData(LightingShaderControlledFloat.UOffset, "u_offset")]
        [InlineData(LightingShaderControlledFloat.UScale, "u_scale")]
        [InlineData(LightingShaderControlledFloat.VOffset, "v_offset")]
        [InlineData(LightingShaderControlledFloat.VScale, "v_scale")]
        public void ForShader_LightingFloatVariable_MapsUvOffsetsAndScales(LightingShaderControlledFloat variable, string expected)
        {
            var scene = new ControllerScene();
            int shader = scene.AddLightingShader();
            scene.AddController<BSLightingShaderPropertyFloatController>(shader, scene.AddFloatInterpolator(KeyType.LINEAR_KEY, new(0f, 0f), new(50f, 5f)), c => c.ControlledVariable = variable);

            var track = Assert.Single(ShaderControllerTracks.ForShader(scene.Nif, shader).Tracks);

            Assert.Equal(expected, track.Target);
            Assert.Equal(5f, track.Values[^1][0]);
        }

        [Theory]
        [InlineData(0f, 0f)]
        [InlineData(5f, 1f)]
        [InlineData(0f, 1f)]
        [InlineData(float.MaxValue, -float.MaxValue)]
        [InlineData(float.NaN, float.NaN)]
        public void ForShader_RangeThatIsEmptyInvertedOrNotCoveringTheKeys_UsesTheKeyRangeAndCountsIt(float start, float stop)
        {
            var scene = new ControllerScene();
            int shader = scene.AddEffectShader();
            scene.AddController<BSEffectShaderPropertyFloatController>(shader, scene.AddFloatInterpolator(KeyType.LINEAR_KEY, new(0.5f, 1f), new(2f, 3f)), c =>
            {
                c.ControlledVariable = EffectShaderControlledVariable.AlphaTransparency;
                c.StartTime = start;
                c.StopTime = stop;
            });

            var track = Assert.Single(ShaderControllerTracks.ForShader(scene.Nif, shader).Tracks);

            Assert.Equal((0.5f, 2f), (track.Start, track.Stop));
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(ClipTiming.RangeReplacedCounter, StringComparison.Ordinal));
        }

        [Fact]
        public void ForShader_RangeCoveringTheKeys_IsKept()
        {
            var scene = new ControllerScene();
            int shader = scene.AddEffectShader();
            scene.AddController<BSEffectShaderPropertyFloatController>(shader, scene.AddFloatInterpolator(KeyType.LINEAR_KEY, new(0.5f, 1f), new(2f, 3f)), c =>
            {
                c.ControlledVariable = EffectShaderControlledVariable.AlphaTransparency;
                c.StartTime = 0f;
                c.StopTime = 10f;
            });

            var track = Assert.Single(ShaderControllerTracks.ForShader(scene.Nif, shader).Tracks);

            Assert.Equal((0f, 10f), (track.Start, track.Stop));
        }

        [Fact]
        public void ForShader_ConstantWithUnusableRange_SitsAtTheFirstFiniteTime()
        {
            var scene = new ControllerScene();
            int shader = scene.AddEffectShader();
            scene.AddController<BSEffectShaderPropertyFloatController>(shader, scene.AddConstantInterpolator(0.75f), c =>
            {
                c.ControlledVariable = EffectShaderControlledVariable.AlphaTransparency;
                c.StartTime = float.NaN;
                c.StopTime = float.NaN;
            });

            var track = Assert.Single(ShaderControllerTracks.ForShader(scene.Nif, shader).Tracks);

            Assert.Equal((0f, 0f), (track.Start, track.Stop));
            Assert.Equal(new[] { 0f }, track.Times);
            Assert.Equal(0.75f, Assert.Single(track.Values)[0]);
        }

        [Fact]
        public void ForShader_NonFiniteKey_DropsTheControllerAndCountsItInsteadOfFailingTheFile()
        {
            var scene = new ControllerScene();
            int shader = scene.AddEffectShader();
            scene.AddEffectFloatController(shader, EffectShaderControlledVariable.AlphaTransparency, scene.AddFloatInterpolator(KeyType.LINEAR_KEY, new(0f, 1f), new(1f, float.NaN)));

            var result = ShaderControllerTracks.ForShader(scene.Nif, shader);

            Assert.Empty(result.Tracks);
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(ControllerSources.NonFiniteKeysCounter, StringComparison.Ordinal));
        }

        [Fact]
        public void ForShader_EffectColorController_ProducesThreeComponentEmissiveColor()
        {
            var scene = new ControllerScene();
            int shader = scene.AddEffectShader();
            scene.AddController<BSEffectShaderPropertyColorController>(shader,
                scene.AddPoint3Interpolator(KeyType.LINEAR_KEY, new(0f, new Vector3(0.25f, 0.5f, 0.75f)), new(2f, new Vector3(1f, 0.5f, 0f))),
                c => c.ControlledColor = EffectShaderControlledColor.EmissiveColor);

            var track = Assert.Single(ShaderControllerTracks.ForShader(scene.Nif, shader).Tracks);

            Assert.Equal("emissive_color", track.Target);
            Assert.Equal(new[] { 0.25f, 0.5f, 0.75f }, track.Values[0]);
            Assert.Equal(new[] { 1f, 0.5f, 0f }, track.Values[1]);
        }

        [Fact]
        public void ForShader_UnsupportedControllers_AreCountedAndReportedNotDropped()
        {
            var scene = new ControllerScene();
            int shader = scene.AddEffectShader();
            var interpolator = scene.AddFloatInterpolator(KeyType.LINEAR_KEY, new(0f, 1f), new(1f, 2f));
            scene.AddEffectFloatController(shader, EffectShaderControlledVariable.FalloffStartAngle, interpolator);
            scene.AddEffectFloatController(shader, EffectShaderControlledVariable.AlphaTransparency, interpolator);

            var result = ShaderControllerTracks.ForShader(scene.Nif, shader);

            Assert.Equal("alpha", Assert.Single(result.Tracks).Target);
            const string description = "BSEffectShaderPropertyFloatController variable FalloffStartAngle";
            Assert.Equal(description, Assert.Single(result.Unsupported));
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(NifAnimationStats.UnsupportedPrefix + description + ": ", StringComparison.Ordinal));
        }

        [Theory]
        [InlineData(0x8000u, true)]
        [InlineData(0x1u, false)]
        public void HasDedicatedExtractor_LightingShaderController_DependsOnTheShaderBeingARefractionShader(uint flags1, bool expected)
        {
            var scene = new ControllerScene();
            var refractionShader = EffectShaderFixture.CreateRefraction(flags1, 0, 0f, -1);
            refractionShader.Controller = new NiBlockRef<NiTimeController>(-1);
            int shader = scene.Nif.AddBlock(refractionShader);
            int controller = scene.AddController<BSLightingShaderPropertyFloatController>(shader, scene.AddFloatInterpolator(KeyType.LINEAR_KEY, new(0f, 0f), new(1f, 1f)),
                c => c.Target = new NiBlockPtr<NiObjectNET>(shader));

            Assert.Equal(expected, NifControllerExtractor.HasDedicatedExtractor(scene.Nif, (NiTimeController)scene.Nif.Blocks[controller]));
        }

        [Fact]
        public void HasDedicatedExtractor_EffectLightAndParticleControllers_AreExportedElsewhereAndOtherControllersAreNot()
        {
            var scene = new ControllerScene();
            var dedicated = new NiTimeController[] { new BSEffectShaderPropertyFloatController(), new NiLightDimmerController(), new NiPSysEmitterCtlr(), new NiPSysUpdateCtlr() };

            Assert.All(dedicated, c => Assert.True(NifControllerExtractor.HasDedicatedExtractor(scene.Nif, c)));
            Assert.False(NifControllerExtractor.HasDedicatedExtractor(scene.Nif, new NiVisController()));
        }

        [Fact]
        public void ForShader_LightingColorController_IsUnsupported()
        {
            var scene = new ControllerScene();
            int shader = scene.AddLightingShader();
            scene.AddController<BSLightingShaderPropertyColorController>(shader, scene.AddPoint3Interpolator(KeyType.LINEAR_KEY, new(0f, Vector3.One), new(1f, Vector3.Zero)));

            var result = ShaderControllerTracks.ForShader(scene.Nif, shader);

            Assert.Empty(result.Tracks);
            Assert.Single(result.Unsupported);
        }

        [Fact]
        public void ForShader_TwoClampSequences_TakeTheFirstAndCountTheTie()
        {
            var scene = new ControllerScene();
            int shader = scene.AddEffectShader();
            int controller = scene.AddEffectFloatController(shader, EffectShaderControlledVariable.AlphaTransparency);
            scene.AddSequence("first", CycleType.CYCLE_CLAMP, 0f, 1f, (controller, scene.AddFloatInterpolator(KeyType.LINEAR_KEY, new(0f, 0.1f), new(1f, 0.2f))));
            scene.AddSequence("second", CycleType.CYCLE_CLAMP, 0f, 1f, (controller, scene.AddFloatInterpolator(KeyType.LINEAR_KEY, new(0f, 0.9f), new(1f, 0.8f))));

            var track = Assert.Single(ShaderControllerTracks.ForShader(scene.Nif, shader).Tracks);

            Assert.Equal("clamp", track.Cycle);
            Assert.Equal(0.1f, track.Values[0][0], Tolerance);
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(ShaderControllerTracks.SequenceTiesCounter, StringComparison.Ordinal));
        }

        [Fact]
        public void ForShader_StandaloneController_UsesItsOwnTimingAndCycle()
        {
            var scene = new ControllerScene();
            int shader = scene.AddEffectShader();
            scene.AddController<BSEffectShaderPropertyFloatController>(shader, scene.AddFloatInterpolator(KeyType.LINEAR_KEY, new(0f, 0f), new(4f, 1f)), c =>
            {
                c.ControlledVariable = EffectShaderControlledVariable.UOffset;
                c.StopTime = 4f;
                c.Frequency = 2f;
                c.Phase = 0.5f;
                c.Flags.CycleType = CycleType.CYCLE_CLAMP;
            });

            var track = Assert.Single(ShaderControllerTracks.ForShader(scene.Nif, shader).Tracks);

            Assert.Equal(("clamp", 0f, 4f, 2f, 0.5f), (track.Cycle, track.Start, track.Stop, track.Frequency, track.Phase));
        }

        [Fact]
        public void ForShader_ManagerControlledControllerWithoutSequenceData_IsDroppedAndCounted()
        {
            var scene = new ControllerScene();
            int shader = scene.AddEffectShader();
            scene.AddEffectFloatController(shader, EffectShaderControlledVariable.AlphaTransparency);

            var result = ShaderControllerTracks.ForShader(scene.Nif, shader);

            Assert.Empty(result.Tracks);
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(ShaderControllerTracks.WithoutKeysCounter, StringComparison.Ordinal));
        }

        [Fact]
        public void ForShader_ConstantInterpolatorWithoutKeys_BecomesOneKeyTrack()
        {
            var scene = new ControllerScene();
            int shader = scene.AddEffectShader();
            scene.AddEffectFloatController(shader, EffectShaderControlledVariable.EmissiveMultiple, scene.AddConstantInterpolator(2.5f));

            var track = Assert.Single(ShaderControllerTracks.ForShader(scene.Nif, shader).Tracks);

            Assert.Equal(2.5f, Assert.Single(track.Values)[0]);
        }

        [Fact]
        public void ForLight_DiffuseColorController_IsMappedAndAmbientIsUnsupported()
        {
            var scene = new ControllerScene();
            int light = scene.AddPointLight();
            int colors = scene.AddPoint3Interpolator(KeyType.LINEAR_KEY, new(0f, new Vector3(1f, 0.5f, 0.25f)), new(1f, Vector3.Zero));
            scene.AddController<NiLightColorController>(light, colors, c => c.TargetColor = LightColor.LC_DIFFUSE);
            scene.AddController<NiLightColorController>(light, colors, c => c.TargetColor = LightColor.LC_AMBIENT);

            var result = ShaderControllerTracks.ForLight(scene.Nif, light);

            Assert.Equal("light_color", Assert.Single(result.Tracks).Target);
            Assert.Single(result.Unsupported);
        }

        [Fact]
        public void ForLight_TransformControllerOnTheLightItself_IsCountedUnsupportedNotIgnored()
        {
            var scene = new ControllerScene();
            int light = scene.AddPointLight();
            scene.AddController<NiTransformController>(light, scene.AddBlendInterpolator());

            var result = ShaderControllerTracks.ForLight(scene.Nif, light);

            Assert.Empty(result.Tracks);
            const string description = "NiTransformController on the light block";
            Assert.Equal(description, Assert.Single(result.Unsupported));
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(NifAnimationStats.UnsupportedPrefix + description + ": ", StringComparison.Ordinal));
        }

        [Fact]
        public void ForShaderAndForLight_BlockOfTheWrongKind_Throw()
        {
            var scene = new ControllerScene();
            int shader = scene.AddEffectShader();

            Assert.Throws<NifAnimationException>(() => ShaderControllerTracks.ForShader(scene.Nif, 999));
            Assert.Throws<NifAnimationException>(() => ShaderControllerTracks.ForLight(scene.Nif, shader));
        }

        private static float ValueAt(ControllerTrack track, float time)
        {
            int index = track.Times.Select((t, i) => (t, i)).OrderBy(p => MathF.Abs(p.t - time)).First().i;
            Assert.True(MathF.Abs(track.Times[index] - time) < 1e-3f, $"no sample near {time}");
            return track.Values[index][0];
        }

        private readonly record struct ScalarKey(float Time, float Value);

        private readonly record struct VectorKeyValue(float Time, Vector3 Value);

        private sealed class ControllerScene
        {
            public NifFile Nif { get; } = new();

            public ControllerScene()
            {
                Nif.Create(NiVersion.GetFO4(), true);
            }

            public int AddEffectShader() => Nif.AddBlock(new BSEffectShaderProperty { Controller = new NiBlockRef<NiTimeController>(-1) });

            public int AddLightingShader() => Nif.AddBlock(new BSLightingShaderProperty { Controller = new NiBlockRef<NiTimeController>(-1) });

            public int AddPointLight() => Nif.AddBlock(new NiPointLight { Controller = new NiBlockRef<NiTimeController>(-1) });

            public int AddEffectFloatController(int shader, EffectShaderControlledVariable variable, int? interpolator = null)
                => AddController<BSEffectShaderPropertyFloatController>(shader, interpolator ?? AddBlendInterpolator(), c => c.ControlledVariable = variable);

            public int AddController<T>(int owner, int interpolator, Action<T>? configure = null) where T : NiSingleInterpController, new()
            {
                var controller = new T { StartTime = 0f, StopTime = 1f, Frequency = 1f, Flags = new TimeControllerFlags(), NextController = new NiBlockRef<NiTimeController>(-1) };
                controller.Interpolator = new NiBlockRef<NiInterpolator>(interpolator);
                configure?.Invoke(controller);
                int index = Nif.AddBlock(controller);
                var ownerBlock = (NiObjectNET)Nif.Blocks[owner];
                if (ownerBlock.Controller.Index < 0)
                {
                    ownerBlock.Controller = new NiBlockRef<NiTimeController>(index);
                    return index;
                }
                var tail = (NiTimeController)Nif.Blocks[ownerBlock.Controller.Index];
                while (tail.NextController.Index >= 0)
                    tail = (NiTimeController)Nif.Blocks[tail.NextController.Index];
                tail.NextController = new NiBlockRef<NiTimeController>(index);
                return index;
            }

            public int AddBlendInterpolator() => Nif.AddBlock(new NiBlendFloatInterpolator { Value = UnsetValue });

            public int AddConstantInterpolator(float value) => Nif.AddBlock(new NiFloatInterpolator { Value = value, Data = new NiBlockRef<NiFloatData>(-1) });

            public int AddFloatInterpolator(KeyType type, params ScalarKey[] keys)
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
                var interpolator = new NiFloatInterpolator
                {
                    Value = UnsetValue,
                    Data = new NiBlockRef<NiFloatData>(Nif.AddBlock(data)),
                };
                return Nif.AddBlock(interpolator);
            }

            public int AddPoint3Interpolator(KeyType type, params VectorKeyValue[] keys)
            {
                var data = new NiPosData
                {
                    Data = new KeyGroup<Vector3>
                    {
                        Interpolation = type,
                        NumKeys = (uint)keys.Length,
                        Keys = keys.Select(k => new Key<Vector3> { Time = k.Time, Value = k.Value }).ToList(),
                    },
                };
                var interpolator = new NiPoint3Interpolator
                {
                    Value = new Vector3(UnsetValue),
                    Data = new NiBlockRef<NiPosData>(Nif.AddBlock(data)),
                };
                return Nif.AddBlock(interpolator);
            }

            public int AddSequence(string name, CycleType cycle, float start, float stop, params (int Controller, int Interpolator)[] blocks)
            {
                var sequence = new NiControllerSequence
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
                };
                return Nif.AddBlock(sequence);
            }
        }
    }
}
