using System.Drawing;
using System.Text.Json.Nodes;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Mutagen.Bethesda.Plugins.Records;
using Noggog;

namespace AssetConverter
{
    /// <summary>
    /// Plain-data projection of every property of a Mutagen WATR record. Property names are Mutagen's, nested structures are nested objects, colours
    /// are <c>[r,g,b,a]</c> bytes, form links are form key strings, enums are names, flags are name
    /// arrays, raw byte blobs are byte arrays. Unreadable subrecords are tallied loudly in
    /// <see cref="ReadFailureTally"/> and left out of the object rather than written as defaults.
    /// </summary>
    public static class WaterJson
    {
        public static JsonObject Project(IWaterGetter w)
        {
            var o = new Fields("WATR", w.EditorID ?? w.FormKey.ToString());
            o.Set("Name", () => w.Name?.String);
            o.Set("Opacity", () => (JsonNode?)w.Opacity);
            o.Set("Flags", () => FlagNames(w.Flags));
            o.Set("Material", () => Link(w.Material));
            o.Set("OpenSound", () => Link(w.OpenSound));
            o.Set("ConsumeSpell", () => Link(w.ConsumeSpell));
            o.Set("ContactSpell", () => Link(w.ContactSpell));
            o.Set("ImageSpace", () => Link(w.ImageSpace));
            o.Set("DATA", () => Bytes(w.DATA));
            o.SetFloat("FogDepthAmount", () => w.FogDepthAmount);
            o.Set("FogShallowColor", () => Rgba(w.FogShallowColor));
            o.Set("FogDeepColor", () => Rgba(w.FogDeepColor));
            o.SetFloat("FogColorShallowRange", () => w.FogColorShallowRange);
            o.SetFloat("FogColorDeepRange", () => w.FogColorDeepRange);
            o.SetFloat("FogShallowAlpha", () => w.FogShallowAlpha);
            o.SetFloat("FogDeepAlpha", () => w.FogDeepAlpha);
            o.SetFloat("FogAlphaShallowRange", () => w.FogAlphaShallowRange);
            o.SetFloat("FogAlphaDeepRange", () => w.FogAlphaDeepRange);
            o.Set("FogUnderwaterColor", () => Rgba(w.FogUnderwaterColor));
            o.SetFloat("FogUnderwaterAmount", () => w.FogUnderwaterAmount);
            o.SetFloat("FogUnderwaterNear", () => w.FogUnderwaterNear);
            o.SetFloat("FogUnderwaterFar", () => w.FogUnderwaterFar);
            o.SetFloat("PhysicalNormalMagnitude", () => w.PhysicalNormalMagnitude);
            o.SetFloat("PhysicalShallowNormalFalloff", () => w.PhysicalShallowNormalFalloff);
            o.SetFloat("PhysicalDeepNormalFalloff", () => w.PhysicalDeepNormalFalloff);
            o.SetFloat("PhysicalReflectivityAmount", () => w.PhysicalReflectivityAmount);
            o.SetFloat("PhysicalFresnelAmount", () => w.PhysicalFresnelAmount);
            o.SetFloat("PhysicalSurfaceEffectFalloff", () => w.PhysicalSurfaceEffectFalloff);
            o.SetFloat("PhysicalDisplacementSimulatorForce", () => w.PhysicalDisplacementSimulatorForce);
            o.SetFloat("PhysicalDisplacementSimulatorVelocity", () => w.PhysicalDisplacementSimulatorVelocity);
            o.SetFloat("PhysicalDisplacementSimulatorFalloff", () => w.PhysicalDisplacementSimulatorFalloff);
            o.SetFloat("PhysicalDisplacementSimulatorDampener", () => w.PhysicalDisplacementSimulatorDampener);
            o.SetFloat("PhysicalDisplacementSimulatorStartingSize", () => w.PhysicalDisplacementSimulatorStartingSize);
            o.Set("PhysicalReflectionColor", () => Rgba(w.PhysicalReflectionColor));
            o.SetFloat("SpecularSunSpecularPower", () => w.SpecularSunSpecularPower);
            o.SetFloat("SpecularSunSpecularMagnitude", () => w.SpecularSunSpecularMagnitude);
            o.SetFloat("SpecularSunSparklePower", () => w.SpecularSunSparklePower);
            o.SetFloat("SpecularSunSparkleMagnitude", () => w.SpecularSunSparkleMagnitude);
            o.SetFloat("SpecularInteriorSpecularRadius", () => w.SpecularInteriorSpecularRadius);
            o.SetFloat("SpecularInteriorSpecularBrightness", () => w.SpecularInteriorSpecularBrightness);
            o.SetFloat("SpecularInteriorSpecularPower", () => w.SpecularInteriorSpecularPower);
            o.SetFloat("SiltAmount", () => w.SiltAmount);
            o.Set("SiltLightColor", () => Rgba(w.SiltLightColor));
            o.Set("SiltDarkColor", () => Rgba(w.SiltDarkColor));
            o.Set("ScreenSpaceReflections", () => (JsonNode?)w.ScreenSpaceReflections);
            o.Set("NoiseLayerOne", () => NoiseLayer(w.NoiseLayerOne));
            o.Set("NoiseLayerTwo", () => NoiseLayer(w.NoiseLayerTwo));
            o.Set("NoiseLayerThree", () => NoiseLayer(w.NoiseLayerThree));
            o.Set("GNAM", () => Bytes(w.GNAM));
            o.Set("LinearVelocity", () => Vector(w.LinearVelocity));
            o.Set("AngularVelocity", () => Vector(w.AngularVelocity));
            o.Set("DNAMDataTypeState", () => FlagNames(w.DNAMDataTypeState));
            return o.Json;
        }

        static JsonObject NoiseLayer(IWaterNoisePropertiesGetter n)
        {
            var o = new Fields("WATR noise layer", "");
            o.SetFloat("WindDirection", () => n.WindDirection);
            o.SetFloat("WindSpeed", () => n.WindSpeed);
            o.SetFloat("AmplitudeScale", () => n.AmplitudeScale);
            o.SetFloat("UvScale", () => n.UvScale);
            o.SetFloat("NoiseFalloff", () => n.NoiseFalloff);
            o.Set("Texture", () => n.Texture);
            return o.Json;
        }

        static JsonNode? Link<T>(IFormLinkNullableGetter<T> link) where T : class, IMajorRecordGetter =>
            link.IsNull ? null : link.FormKey.ToString();

        public static JsonArray Rgba(Color c) => new(c.R, c.G, c.B, c.A);

        static JsonArray? Vector(P3Float? v) => v is { } p ? new JsonArray(p.X, p.Y, p.Z) : null;

        static JsonArray? Bytes(ReadOnlyMemorySlice<byte>? b)
        {
            if (b is not { } slice) return null;
            var arr = new JsonArray();
            foreach (var x in slice.Span) arr.Add((int)x);
            return arr;
        }

        /// <summary>Flag names; bits no enum member names are kept as one trailing <c>0x..</c> hex string.</summary>
        static JsonArray FlagNames<TEnum>(TEnum value) where TEnum : struct, Enum
        {
            var names = new JsonArray();
            ulong remaining = Convert.ToUInt64(value, System.Globalization.CultureInfo.InvariantCulture);
            foreach (var flag in Enum.GetValues<TEnum>())
            {
                ulong bit = Convert.ToUInt64(flag, System.Globalization.CultureInfo.InvariantCulture);
                if (bit == 0 || (remaining & bit) != bit) continue;
                names.Add(flag.ToString());
                remaining &= ~bit;
            }
            if (remaining != 0) names.Add($"0x{remaining:X}");
            return names;
        }

        /// <summary>Collects projected fields of one record; a read failure is tallied against "<record> <field>".</summary>
        sealed class Fields(string record, string subject)
        {
            public JsonObject Json { get; } = new();

            public void Set(string name, Func<JsonNode?> read)
            {
                JsonNode? value = ReadFailureTally.Shared.Read($"{record} {name}", subject, read, null);
                if (value != null) Json[name] = value;
            }

            /// <summary>A failed read is already tallied by <c>Read</c>; a finite value is stored, a genuine NaN/Infinity is tallied and omitted.</summary>
            public void SetFloat(string name, Func<float> read)
            {
                string site = $"{record} {name}";
                float? value = ReadFailureTally.Shared.Read<float?>(site, subject, () => read(), null);
                if (value is not { } v) return;
                if (float.IsFinite(v)) Json[name] = v;
                else ReadFailureTally.Shared.Record(site, new InvalidDataException($"not a finite float ({v})"), subject);
            }
        }
    }
}
