using System.Text.Json.Nodes;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Noggog;

namespace AssetConverter
{
    /// <summary>
    /// JSON projections of the WTHR fields the engine reads beyond the colour, fog and image space columns:
    /// cloud layers, the DATA block, the weather sound list and the god rays per time of day. Property names are snake_case, form links are form key strings,
    /// colours are <c>[r,g,b,a]</c> bytes and the time-of-day slots use the names of the <c>weather.colors</c> column.
    /// </summary>
    internal static class WeatherJson
    {
        // Mutagen exposes these DATA bytes as raw / divisor (wind direction, range) or raw * multiplier (transition delta).
        public const double WindDirectionDivisor = 360.0;
        public const double WindDirectionRangeDivisor = 180.0;
        public const double TransDeltaMultiplier = 4.0;

        public static byte RawWindDirection(float windDirection) => ToByte(windDirection * WindDirectionDivisor);

        public static byte RawWindDirectionRange(float windDirectionRange) => ToByte(windDirectionRange * WindDirectionRangeDivisor);

        public static byte RawTransDelta(float transDelta) => ToByte(transDelta / TransDeltaMultiplier);

        private static byte ToByte(double value) => (byte)Math.Round(value);

        public static JsonObject? Clouds(IWeatherGetter weather)
        {
            var layers = weather.Clouds;
            var textures = weather.CloudTextures;
            if (layers.Count == 0 && textures.Length == 0) return null;

            var textureArray = new JsonArray();
            foreach (string? texture in textures.ToArray())
                textureArray.Add(string.IsNullOrEmpty(texture) ? null : texture);

            var layerArray = new JsonArray();
            foreach (var layer in layers)
                layerArray.Add(Layer(layer));

            return new JsonObject
            {
                ["textures"] = textureArray,
                ["layers"] = layerArray
            };
        }

        // A layer authored without its colour or alpha subrecords has no values for them; they stay JSON null instead of invented defaults.
        private static JsonObject Layer(ICloudLayerGetter layer) => new()
        {
            ["enabled"] = layer.Enabled,
            ["x_speed"] = layer.XSpeed,
            ["y_speed"] = layer.YSpeed,
            ["colors"] = layer.Colors is { } colors
                ? ByTimeOfDay(time => WaterJson.Rgba(WeatherProcessor.Slot(colors, time)))
                : null,
            ["alphas"] = layer.Alphas is { } alphas
                ? ByTimeOfDay(time => JsonValue.Create(Alpha(alphas, time)))
                : null
        };

        private static JsonObject ByTimeOfDay(Func<string, JsonNode?> select)
        {
            var slots = new JsonObject();
            foreach (string time in WeatherProcessor.TimesOfDay)
                slots[time] = select(time);
            return slots;
        }

        public static JsonObject? GodRays(IWeatherGetter weather) =>
            weather.GodRays is { } godRays
                ? ByTimeOfDay(time => LinkKey.Text(GodRaysSlot(godRays, time)))
                : null;

        private static IFormLinkGetter<IGodRaysGetter> GodRaysSlot(IWeatherGodRaysGetter godRays, string time) => time switch
        {
            "Sunrise" => godRays.GodRaySunrise,
            "Day" => godRays.GodRayDay,
            "Sunset" => godRays.GodRaySunset,
            "Night" => godRays.GodRayNight,
            "EarlySunrise" => godRays.GodRayEarlySunrise,
            "LateSunrise" => godRays.GodRayLateSunrise,
            "EarlySunset" => godRays.GodRayEarlySunset,
            "LateSunset" => godRays.GodRayLateSunset,
            _ => throw new ArgumentOutOfRangeException(nameof(time), time, "Unknown time of day")
        };

        private static float Alpha(IWeatherAlphaGetter alphas, string time) => time switch
        {
            "Sunrise" => alphas.Sunrise,
            "Day" => alphas.Day,
            "Sunset" => alphas.Sunset,
            "Night" => alphas.Night,
            "EarlySunrise" => alphas.EarlySunrise,
            "LateSunrise" => alphas.LateSunrise,
            "EarlySunset" => alphas.EarlySunset,
            "LateSunset" => alphas.LateSunset,
            _ => throw new ArgumentOutOfRangeException(nameof(time), time, "Unknown time of day")
        };

        public static JsonObject Data(IWeatherGetter weather)
        {
            var lightning = weather.LightningColor;
            return new JsonObject
            {
                ["wind_speed"] = weather.WindSpeed.Value,
                ["wind_turbulence"] = WindTurbulence(weather),
                ["wind_direction_raw"] = RawWindDirection(weather.WindDirection),
                ["wind_direction_range_raw"] = RawWindDirectionRange(weather.WindDirectionRange),
                ["trans_delta_raw"] = RawTransDelta(weather.TransDelta),
                ["sun_glare"] = weather.SunGlare.Value,
                ["sun_damage"] = weather.SunDamage.Value,
                ["precipitation_begin_fade_in"] = weather.PrecipitationBeginFadeIn.Value,
                ["precipitation_end_fade_out"] = weather.PrecipitationEndFadeOut.Value,
                ["thunder_begin_fade_in"] = weather.ThunderLightningBeginFadeIn.Value,
                ["thunder_end_fade_out"] = weather.ThunderLightningEndFadeOut.Value,
                ["thunder_frequency"] = weather.ThunderLightningFrequency.Value,
                ["lightning_color"] = new JsonArray(lightning.R, lightning.G, lightning.B),
                ["flags"] = (int)weather.Flags,
                ["visual_effect_begin"] = weather.VisualEffectBegin.Value,
                ["visual_effect_end"] = weather.VisualEffectEnd.Value,
                ["volatility_mult"] = weather.VolatilityMult,
                ["visibility_mult"] = weather.VisibilityMult
            };
        }

        private static double? WindTurbulence(IWeatherGetter weather) =>
            weather.DATADataTypeState.HasFlag(Weather.DATADataType.Break0) ? null : weather.WindTurbulance.Value;

        // The stored value is a plain enumeration; Mutagen's flag-valued enum names do not match it.
        public static string SoundKind(uint raw, string subject) => raw switch
        {
            0 => "default",
            1 => "precipitation",
            2 => "wind",
            3 => "thunder",
            _ => throw new UnsupportedEsmDataException($"{subject}: unknown weather sound type {raw}.")
        };

        public static JsonArray Sounds(IWeatherGetter weather)
        {
            string subject = $"WTHR {weather.FormKey} ({weather.EditorID})";
            var sounds = new JsonArray();
            foreach (var sound in weather.Sounds)
                sounds.Add(new JsonObject { ["kind"] = SoundKind((uint)sound.Type, subject), ["sound"] = LinkKey.Text(sound.Sound) });
            return sounds;
        }
    }
}
