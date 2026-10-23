using System.Text.Json.Nodes;
using NiflySharp;
using NiflySharp.Blocks;

namespace AssetConverter
{
    // Timing, key lists and controller-chain walking for the controllers on one particle system.
    // Key data resolution (own interpolator, else the looping controller-sequence block) is shared with the
    // shader and light controllers in ControllerSources.Resolve. A controller holding a non-finite number is
    // left out and counted; the rest of the system is kept.
    internal sealed class SystemControllers
    {
        public const string SequenceTiesCounter = "particle controllers with several equally ranked sequences (first in file order used)";

        private readonly NifFile _nif;
        private readonly Dictionary<string, JsonObject> _emitters = new(StringComparer.Ordinal);
        private readonly List<JsonObject> _fieldTracks = new();
        private readonly Dictionary<string, JsonObject> _activeTracks = new(StringComparer.Ordinal);

        private SystemControllers(NifFile nif) => _nif = nif;

        public IReadOnlyList<JsonObject> FieldTracks => _fieldTracks;

        public IReadOnlyDictionary<string, JsonObject> ActiveTracks => _activeTracks;

        public float UpdateFrequency { get; private set; } = 1f;

        public float UpdatePhase { get; private set; }

        public JsonObject EmitterController(string emitterName)
        {
            if (_emitters.TryGetValue(emitterName, out var resolved))
                return resolved.DeepClone().AsObject();
            NifAnimationStats.Add(ParticleSystemExtras.EmitterWithoutControllerCounter);
            return TimingJson(new ClipTiming(NifControllerExtractor.CycleClamp, 0f, 0f, 1f, 0f), NeverEmitting());
        }

        public static SystemControllers Read(NifFile nif, NiParticleSystem system)
        {
            var result = new SystemControllers(nif);
            foreach (var (index, controller) in ShaderControllerTracks.ControllerChain(nif, system))
                result.Add(index, controller);
            return result;
        }

        private void Add(int index, NiTimeController controller)
        {
            try
            {
                AddSupported(index, controller);
            }
            catch (NonFiniteParticleValueException)
            {
                NifAnimationStats.Add(ParticleSystemExtras.NonFiniteControllerCounter);
            }
        }

        private void AddSupported(int index, NiTimeController controller)
        {
            switch (controller)
            {
                case NiPSysUpdateCtlr:
                    ParticleSystemExtras.Num(controller.Frequency);
                    ParticleSystemExtras.Num(controller.Phase);
                    UpdateFrequency = controller.Frequency;
                    UpdatePhase = controller.Phase;
                    break;
                case NiPSysEmitterCtlr emitterController:
                    AddEmitterController(index, emitterController);
                    break;
                case NiPSysModifierActiveCtlr activeController:
                    AddActiveController(index, activeController);
                    break;
                case NiPSysModifierFloatCtlr floatController when ParticleSystemExtras.TryGetFieldName(controller, out var field):
                    AddFieldTrack(index, floatController, field);
                    break;
                default:
                    NifAnimationStats.AddUnsupported($"particle controller {controller.GetType().Name}");
                    break;
            }
        }

        private void AddEmitterController(int index, NiPSysEmitterCtlr controller)
        {
            var rate = Resolve(index, controller, controller.Interpolator.Index, ControllerSources.ReadFloat);
            var active = Resolve(index, controller, controller.VisibilityInterpolator.Index, ControllerSources.ReadBool);
            if (rate != null && active != null && rate.Timing != active.Timing)
                NifAnimationStats.Add(ParticleSystemExtras.ControllerTimingMismatchCounter);
            _emitters[controller.ModifierName?.String ?? ""] = EmitterJson(rate, active, controller);
        }

        private void AddActiveController(int index, NiPSysModifierActiveCtlr controller)
        {
            var active = Resolve(index, controller, controller.Interpolator.Index, ControllerSources.ReadBool);
            if (active != null)
                _activeTracks[controller.ModifierName?.String ?? ""] = TrackJson(active, value => value);
        }

        private void AddFieldTrack(int index, NiPSysModifierFloatCtlr controller, string field)
        {
            var resolved = Resolve(index, controller, controller.Interpolator.Index, ControllerSources.ReadFloat);
            if (resolved == null)
                return;
            var track = TrackJson(resolved, value => ParticleSystemExtras.ScaleForField(field, value));
            _fieldTracks.Add(new JsonObject { ["target"] = controller.ModifierName?.String ?? "", ["field"] = field, ["track"] = track });
        }

        private ResolvedKeys? Resolve(int controllerIndex, NiTimeController controller, int interpolatorIndex, Func<NifFile, int, KeyList?> read)
        {
            var resolved = ControllerSources.Resolve(_nif, controllerIndex, controller, interpolatorIndex, read, SequenceTiesCounter);
            if (resolved == null && interpolatorIndex >= 0)
                NifAnimationStats.Add(ParticleSystemExtras.ControllerWithoutKeysCounter);
            return resolved;
        }

        private static JsonObject TimingJson(ClipTiming timing, JsonObject target)
        {
            target["cycle"] = timing.Cycle;
            target["start"] = ParticleSystemExtras.Num(timing.Start);
            target["stop"] = ParticleSystemExtras.Num(timing.Stop);
            target["frequency"] = ParticleSystemExtras.Num(timing.Frequency);
            target["phase"] = ParticleSystemExtras.Num(timing.Phase);
            return target;
        }

        private static JsonObject NeverEmitting() => new()
        {
            ["birth_rate"] = new JsonObject { ["constant"] = 0 },
            ["active"] = new JsonObject { ["constant"] = true },
        };

        private static JsonObject TrackJson(ResolvedKeys resolved, Func<float, float> scale)
        {
            var timing = resolved.CoveredTiming();
            var keys = resolved.Keys;
            var times = keys.HasKeys ? keys.Times : new[] { timing.Start };
            var values = keys.HasKeys ? keys.Values.Select(v => v[0]) : new[] { keys.Constant![0] };
            var json = TimingJson(timing, new JsonObject());
            json["times"] = new JsonArray(times.Select(ParticleSystemExtras.Num).ToArray());
            json["values"] = new JsonArray(values.Select(v => ParticleSystemExtras.Num(scale(v))).ToArray());
            return json;
        }

        // The emitter controller carries one timing for the birth rate and the activity: the birth rate's, else the activity's,
        // widened to cover the keys of whichever of the two is keyed.
        private static JsonObject EmitterJson(ResolvedKeys? rate, ResolvedKeys? active, NiTimeController controller)
        {
            var present = new[] { rate, active }.OfType<ResolvedKeys>().ToList();
            if (present.Count == 0)
                present.Add(new ResolvedKeys(KeyList.Fixed(0f), ClipTiming.Of(controller)));
            var keyed = present.Where(p => p.Keys.HasKeys).ToList();
            var ranges = (keyed.Count > 0 ? keyed : present.Take(1)).Select(p => p.KeyRange).ToList();
            var timing = present[0].Timing.CoveringKeys(ranges.Min(r => r.First), ranges.Max(r => r.Last));

            var json = new JsonObject
            {
                ["birth_rate"] = rate == null ? new JsonObject { ["constant"] = 0 } : ParamJson(rate, value => ParticleSystemExtras.Num(value)),
                ["active"] = active == null ? new JsonObject { ["constant"] = true } : ParamJson(active, value => JsonValue.Create(value != 0f)),
            };
            return TimingJson(timing, json);
        }

        private static JsonObject ParamJson(ResolvedKeys resolved, Func<float, JsonNode> constant)
        {
            var keys = resolved.Keys;
            if (!keys.HasKeys)
                return new JsonObject { ["constant"] = constant(keys.Constant![0]) };
            return new JsonObject
            {
                ["times"] = new JsonArray(keys.Times.Select(ParticleSystemExtras.Num).ToArray()),
                ["values"] = new JsonArray(keys.Values.Select(v => ParticleSystemExtras.Num(v[0])).ToArray()),
            };
        }
    }
}
