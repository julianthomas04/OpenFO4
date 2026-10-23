using System.Globalization;
using System.Numerics;
using System.Reflection;
using System.Text.Json.Nodes;
using NiflySharp;
using NiflySharp.Blocks;
using NiflySharp.Enums;

namespace AssetConverter
{
    // Builds the engine's `particle_system` / `master_particle_system` node extras from a NIF.
    // Lengths, radii, speeds and accelerations are scaled to engine units, direction vectors are
    // permuted to engine axes, angles stay radians. Every direction, extent and angle is expressed in
    // the glTF node-local frame (NIF x, y, z -> x, -z, y): declination is measured from the emitter
    // node's local +Y (the NIF +Z), planar angle runs around +Y from +X towards -Z, a box is
    // width (X) by height (Y, the NIF depth) by depth (Z, the NIF height), a cylinder's height is along Y.
    // Modifiers without an engine counterpart (bomb, colliders, recycle bound, parent velocity) are
    // skipped one by one, reported in the result and counted in NifAnimationStats; the system stays.
    // Node references are glTF node names; a reference without an object (a mesh emitter has no emitter
    // object, a force may lack its object) names the particle system's own node, so none is ever empty.
    // A non-finite number inside a controller drops that controller, inside the system itself drops the system.
    internal static class ParticleSystemExtras
    {
        public const int Version = 1;
        public const string SystemSkippedCounter = "particle system";
        public const string ModifierSkippedCounter = "particle modifier";
        public const string MissingObjectCounter = "particle references without an object (the system's own node used)";
        public const string NonFiniteSystemCounter = "particle systems left out because of a non-finite value";
        public const string EmitterWithoutControllerCounter = "particle emitters without a controller (never emit)";
        public const string ControllerWithoutKeysCounter = "particle controllers without key data";
        public const string NonFiniteControllerCounter = "particle controllers left out because of a non-finite value";
        public const string ControllerTimingMismatchCounter = "particle emitter controllers whose birth rate and activity use different timing (birth rate timing used)";
        public const string MeshEmitterVelocityCounter = "particle mesh emitters with a velocity type other than face normals (normals used)";

        private const float Scaling = GameCoordinates.Scaling;
        private const float UnboundedRange = 1e30f;

        // NiflySharp keeps the particle cap of NiGeometryData in a private field without an accessor.
        private static readonly FieldInfo MaxVerticesField = typeof(NiGeometryData).GetField("_bSMaxVertices", BindingFlags.NonPublic | BindingFlags.Instance)
            ?? throw new InvalidOperationException("NiflySharp no longer exposes the particle cap (NiGeometryData._bSMaxVertices)");

        private static readonly Dictionary<Type, string> FieldTrackNames = new()
        {
            [typeof(NiPSysEmitterSpeedCtlr)] = "speed",
            [typeof(NiPSysEmitterLifeSpanCtlr)] = "life_span",
            [typeof(NiPSysEmitterInitialRadiusCtlr)] = "initial_radius",
            [typeof(NiPSysEmitterDeclinationCtlr)] = "declination",
            [typeof(NiPSysEmitterDeclinationVarCtlr)] = "declination_variation",
            [typeof(NiPSysEmitterPlanarAngleCtlr)] = "planar_angle",
            [typeof(NiPSysGravityStrengthCtlr)] = "gravity_strength",
        };

        private static readonly HashSet<string> LengthTrackFields = new() { "speed", "initial_radius", "gravity_strength" };

        private sealed record Frame(NifFile Nif, string SystemNode);

        public static ParticleSystemResult? Build(NifFile nif, int block, int materialIndex)
        {
            if (nif.Blocks[block] is not NiParticleSystem system)
                throw new NifAnimationException($"block {block} is not a NiParticleSystem");
            if (!NifControllerExtractor.TryGetBlock<NiPSysData>(nif, system.Data_NiBlockRef_NPSD.Index, "particle data", out var data))
                return SkipSystem($"block {block} has no NiPSysData");

            try
            {
                return BuildSystem(new Frame(nif, NifNodeNames.Of(nif, block)), system, data, materialIndex);
            }
            catch (NonFiniteParticleValueException)
            {
                NifAnimationStats.Add(NonFiniteSystemCounter);
                return null;
            }
        }

        private static ParticleSystemResult? BuildSystem(Frame frame, NiParticleSystem system, NiPSysData data, int materialIndex)
        {
            var nif = frame.Nif;
            var skipped = new List<string>();
            var emitters = new List<(NiPSysEmitter Block, JsonObject Json)>();
            var modifiers = new List<(NiPSysModifier Block, JsonObject Json)>();
            foreach (int index in system.Modifiers.Indices)
            {
                if (!NifControllerExtractor.TryGetBlock<NiPSysModifier>(nif, index, "particle modifier", out var modifier))
                    continue;
                switch (modifier)
                {
                    case NiPSysAgeDeathModifier or NiPSysSpawnModifier or NiPSysPositionModifier or NiPSysBoundUpdateModifier:
                        break;
                    case NiPSysEmitter emitter:
                        var emitterJson = EmitterJson(frame, emitter);
                        if (emitterJson == null)
                            return SkipSystem($"emitter {Name(emitter)} has an unsupported emission type");
                        emitters.Add((emitter, emitterJson));
                        break;
                    default:
                        var json = ModifierJson(frame, modifier);
                        if (json == null)
                            Skip(skipped, modifier);
                        else
                            modifiers.Add((modifier, json));
                        break;
                }
            }
            if (emitters.Count == 0)
                return SkipSystem($"system {frame.SystemNode} has no emitter");

            var controllers = SystemControllers.Read(nif, system);
            foreach (var (emitter, json) in emitters)
                json["emitter_ctlr"] = controllers.EmitterController(Name(emitter));
            var activeTracks = controllers.ActiveTracks;
            foreach (var (modifier, json) in modifiers)
            {
                if (activeTracks.TryGetValue(Name(modifier), out var track))
                    json["active_track"] = track.DeepClone();
            }
            foreach (string target in activeTracks.Keys.Where(t => emitters.Any(e => Name(e.Block) == t)))
                NifAnimationStats.AddUnsupported($"particle activity controller on emitter {target} (the emitter controller carries emitter activity)");

            var extras = new JsonObject
            {
                ["v"] = Version,
                ["world_space"] = system.WorldSpace ?? false,
                ["max_particles"] = Convert.ToUInt32(MaxVerticesField.GetValue(data), CultureInfo.InvariantCulture),
                ["material"] = materialIndex,
                ["fade"] = new JsonObject
                {
                    ["near_begin"] = Length(system.NearBegin),
                    ["near_end"] = Length(system.NearEnd),
                    ["far_begin"] = Length(system.FarBegin),
                    ["far_end"] = Length(system.FarEnd),
                },
                ["data"] = DataJson(data),
                ["emitters"] = new JsonArray(emitters.Select(e => (JsonNode)e.Json).ToArray()),
                ["modifiers"] = new JsonArray(modifiers.Select(m => (JsonNode)m.Json).ToArray()),
                ["tracks"] = new JsonArray(controllers.FieldTracks.Select(t => (JsonNode)t).ToArray()),
                ["update"] = new JsonObject { ["frequency"] = Num(controllers.UpdateFrequency), ["phase"] = Num(controllers.UpdatePhase) },
            };
            return new ParticleSystemResult(extras, skipped);
        }

        public static JsonObject BuildMaster(NifFile nif, int block)
        {
            if (nif.Blocks[block] is not BSMasterParticleSystem master)
                throw new NifAnimationException($"block {block} is not a BSMasterParticleSystem");
            return new JsonObject
            {
                ["v"] = Version,
                ["max_emitter_objects"] = (int)master.MaxEmitterObjects,
                ["systems"] = new JsonArray(ExistingNodeNames(nif, master.ParticleSystems.Indices).Select(name => (JsonNode)name).ToArray()),
            };
        }

        private static ParticleSystemResult? SkipSystem(string reason)
        {
            NifAnimationStats.AddUnsupported($"{SystemSkippedCounter} ({reason})");
            return null;
        }

        private static void Skip(List<string> skipped, NiPSysModifier modifier)
        {
            string description = modifier.GetType().Name;
            skipped.Add(description);
            NifAnimationStats.AddUnsupported($"{ModifierSkippedCounter} {description}");
        }

        private static JsonObject DataJson(NiPSysData data)
        {
            var rects = data.HasTextureIndices == true ? data.SubtextureOffsets ?? new List<Vector4>() : new List<Vector4>();
            return new JsonObject
            {
                ["aspect_ratio"] = Num(data.AspectRatio),
                ["aspect_flags"] = (uint)(ushort)data.AspectFlags,
                ["speed_to_aspect"] = Array(data.SpeedtoAspectAspect2, data.SpeedtoAspectSpeed1 * Scaling, data.SpeedtoAspectSpeed2 * Scaling),
                ["subtexture_rects"] = new JsonArray(rects.Select(r => (JsonNode)Array(r.X, r.Y, r.Z, r.W)).ToArray()),
            };
        }

        private static JsonObject? EmitterJson(Frame frame, NiPSysEmitter emitter)
        {
            var json = new JsonObject { ["name"] = Name(emitter) };
            switch (emitter)
            {
                case NiPSysCylinderEmitter cylinder:
                    json["type"] = "cylinder";
                    json["radius"] = Length(cylinder.Radius);
                    json["height"] = Length(cylinder.Height);
                    break;
                case NiPSysBoxEmitter box:
                    json["type"] = "box";
                    json["width"] = Length(box.Width);
                    json["height"] = Length(box.Depth);
                    json["depth"] = Length(box.Height);
                    break;
                case NiPSysSphereEmitter sphere:
                    json["type"] = "sphere";
                    json["radius"] = Length(sphere.Radius);
                    break;
                case NiPSysMeshEmitter mesh:
                    string? emission = mesh.EmissionType switch
                    {
                        EmitFrom.EMIT_FROM_VERTICES => "vertices",
                        EmitFrom.EMIT_FROM_FACE_SURFACE => "face_surface",
                        EmitFrom.EMIT_FROM_FACE_CENTER => "face_centre",
                        _ => null,
                    };
                    if (emission == null)
                        return null;
                    if (mesh.InitialVelocityType != VelocityType.VELOCITY_USE_NORMALS)
                        NifAnimationStats.Add(MeshEmitterVelocityCounter);
                    json["type"] = "mesh";
                    json["mesh_nodes"] = new JsonArray(ExistingNodeNames(frame.Nif, mesh.EmitterMeshes.Indices).Select(name => (JsonNode)name).ToArray());
                    json["emission_type"] = emission;
                    break;
                default:
                    return null;
            }
            json["node"] = NodeRef(frame, emitter is NiPSysVolumeEmitter volume ? volume.EmitterObject?.Index ?? -1 : -1);
            json["speed"] = Length(emitter.Speed);
            json["speed_variation"] = Length(emitter.SpeedVariation);
            json["declination"] = Num(emitter.Declination);
            json["declination_variation"] = Num(emitter.DeclinationVariation);
            json["planar_angle"] = Num(emitter.PlanarAngle);
            json["planar_angle_variation"] = Num(emitter.PlanarAngleVariation);
            var color = emitter.InitialColor;
            json["initial_color"] = Array(color.R, color.G, color.B, color.A);
            json["initial_radius"] = Length(MathF.Max(0f, emitter.InitialRadius));
            json["radius_variation"] = Length(emitter.RadiusVariation);
            json["life_span"] = Num(emitter.LifeSpan);
            json["life_span_variation"] = Num(emitter.LifeSpanVariation);
            return json;
        }

        private static JsonObject? ModifierJson(Frame frame, NiPSysModifier modifier)
        {
            var fields = modifier switch
            {
                BSPSysLODModifier lod => new JsonObject
                {
                    ["type"] = "lod",
                    ["begin"] = Num(lod.LODBeginDistance),
                    ["end"] = Num(lod.LODEndDistance),
                    ["end_emit_scale"] = Num(lod.EndEmitScale),
                    ["end_size"] = Num(lod.EndSize),
                },
                BSPSysSimpleColorModifier color => new JsonObject
                {
                    ["type"] = "simple_color",
                    ["fade_in_percent"] = Num(color.FadeInPercent),
                    ["fade_out_percent"] = Num(color.FadeOutPercent),
                    ["color1_end_percent"] = Num(color.Color1EndPercent),
                    // nif.xml names the fields by file order: Color 1 Start is where colour 2 begins, Color 2 Start where colour 3 begins.
                    ["color2_start_percent"] = Num(color.Color1StartPercent),
                    ["color2_end_percent"] = Num(color.Color2EndPercent),
                    ["color3_start_percent"] = Num(color.Color2StartPercent),
                    ["colors"] = new JsonArray(color.Colors.Select(c => (JsonNode)Array(c.R, c.G, c.B, c.A)).ToArray()),
                },
                BSPSysScaleModifier scale => new JsonObject
                {
                    ["type"] = "scale",
                    ["scales"] = new JsonArray((scale.Scales ?? new List<float>()).Select(Num).ToArray()),
                },
                NiPSysGravityModifier gravity when gravity.ForceType is ForceType.FORCE_PLANAR or ForceType.FORCE_SPHERICAL => new JsonObject
                {
                    ["type"] = "gravity",
                    ["object"] = NodeRef(frame, gravity.GravityObject.Index),
                    ["axis"] = Direction(gravity.GravityAxis),
                    ["decay"] = Num(gravity.Decay),
                    ["strength"] = Length(gravity.Strength),
                    ["force"] = gravity.ForceType == ForceType.FORCE_PLANAR ? "planar" : "spherical",
                    ["turbulence"] = Num(gravity.Turbulence),
                    ["turbulence_scale"] = Num(gravity.TurbulenceScale),
                    ["world_aligned"] = gravity.WorldAligned ?? false,
                },
                NiPSysDragModifier drag => new JsonObject
                {
                    ["type"] = "drag",
                    ["object"] = NodeRef(frame, drag.DragObject.Index),
                    ["axis"] = Direction(drag.DragAxis),
                    ["percentage"] = Num(drag.Percentage),
                    ["range"] = drag.Range >= UnboundedRange ? null : Length(drag.Range),
                    ["range_falloff"] = Num(drag.RangeFalloff),
                },
                BSWindModifier wind => new JsonObject { ["type"] = "wind", ["strength"] = Num(wind.Strength) },
                BSPSysSubTexModifier subTex => new JsonObject
                {
                    ["type"] = "sub_tex",
                    ["start_frame"] = Num(subTex.StartFrame),
                    ["start_frame_fudge"] = Num(subTex.StartFrameFudge),
                    ["end_frame"] = Num(subTex.EndFrame),
                    ["loop_start_frame"] = Num(subTex.LoopStartFrame),
                    ["loop_start_frame_fudge"] = Num(subTex.LoopStartFrameFudge),
                    ["frame_count"] = Num(subTex.FrameCount),
                    ["frame_count_fudge"] = Num(subTex.FrameCountFudge),
                },
                NiPSysRotationModifier rotation => new JsonObject
                {
                    ["type"] = "rotation",
                    ["speed"] = Num(rotation.RotationSpeed),
                    ["speed_variation"] = Num(rotation.RotationSpeedVariation),
                    ["angle"] = Num(rotation.RotationAngle),
                    ["angle_variation"] = Num(rotation.RotationAngleVariation),
                    ["random_speed_sign"] = rotation.RandomRotSpeedSign ?? false,
                    ["random_axis"] = rotation.RandomAxis ?? false,
                    ["axis"] = Direction(rotation.Axis),
                },
                BSPSysInheritVelocityModifier inherit => new JsonObject
                {
                    ["type"] = "inherit_velocity",
                    ["object"] = NodeRef(frame, inherit.InheritObject.Index),
                    ["chance_percent"] = Num(inherit.ChanceToInherit),
                    ["multiplier"] = Num(inherit.VelocityMultiplier),
                    ["variation"] = Num(inherit.VelocityVariation),
                },
                _ => null,
            };
            if (fields == null)
                return null;
            var json = new JsonObject
            {
                ["name"] = Name(modifier),
                ["order"] = (int)modifier.Order,
                ["active"] = modifier.Active ?? true,
            };
            foreach (var (key, value) in fields.ToArray())
            {
                fields.Remove(key);
                json[key] = value;
            }
            return json;
        }

        internal static JsonNode Num(float value)
        {
            if (!float.IsFinite(value))
                throw new NonFiniteParticleValueException(value);
            return JsonValue.Create(value);
        }

        internal static JsonNode Length(float bethesdaUnits) => Num(bethesdaUnits * Scaling);

        private static JsonArray Array(params float[] values) => new(values.Select(Num).ToArray());

        private static JsonArray Direction(Vector3 axis)
        {
            var engine = GameCoordinates.ToGltfDirection(axis);
            return Array(engine.X, engine.Y, engine.Z);
        }

        private static string Name(NiPSysModifier modifier) => modifier.Name?.String ?? "";

        private static string NodeRef(Frame frame, int index)
        {
            if (index >= 0)
                return NifNodeNames.Of(frame.Nif, index);
            NifAnimationStats.Add(MissingObjectCounter);
            return frame.SystemNode;
        }

        private static IEnumerable<string> ExistingNodeNames(NifFile nif, IEnumerable<int> indices)
        {
            foreach (int index in indices)
            {
                if (index >= 0)
                    yield return NifNodeNames.Of(nif, index);
                else
                    NifAnimationStats.Add(MissingObjectCounter);
            }
        }

        internal static float ScaleForField(string field, float value) => LengthTrackFields.Contains(field) ? value * Scaling : value;

        internal static bool TryGetFieldName(NiTimeController controller, out string field) => FieldTrackNames.TryGetValue(controller.GetType(), out field!);
    }
}
