using System.Text.Json;

namespace AssetConverter.Video
{
    internal sealed record AudioTrackInfo(int Id, int SampleRate, int Channels);

    /// <summary>What the decoder's <c>info</c> command states about a video.</summary>
    internal sealed record VideoInfo(int Width, int Height, int FrameCount, int FrameRateNumerator, int FrameRateDenominator, IReadOnlyList<AudioTrackInfo> AudioTracks)
    {
        /// <summary>The bytes of one decoded frame: planar 8-bit 4:2:0.</summary>
        public long FrameBytes => (long)Width * Height * 3 / 2;

        /// <summary>Reads the JSON of <c>info</c>: <c>width</c>, <c>height</c>, <c>frameCount</c>, <c>frameRateNumerator</c>, <c>frameRateDenominator</c> and <c>audioTracks</c> (each with <c>id</c>, <c>sampleRate</c>, <c>channels</c>), in any letter case.</summary>
        /// <exception cref="VideoConversionException">The text is not JSON of that shape, or a value is not positive, or the size is not even (4:2:0).</exception>
        public static VideoInfo Parse(string json)
        {
            try
            {
                using JsonDocument document = JsonDocument.Parse(json);
                JsonElement root = document.RootElement;
                var info = new VideoInfo(
                    Positive(root, "width"), Positive(root, "height"), Positive(root, "frameCount"),
                    Positive(root, "frameRateNumerator"), Positive(root, "frameRateDenominator"),
                    [.. Member(root, "audioTracks").EnumerateArray().Select(track => new AudioTrackInfo(Number(track, "id"), Positive(track, "sampleRate"), Positive(track, "channels")))]);
                if (info.Width % 2 != 0 || info.Height % 2 != 0)
                    throw new VideoConversionException($"The video size {info.Width}x{info.Height} is not even; 4:2:0 frames need an even size.");
                return info;
            }
            catch (Exception ex) when (ex is JsonException or InvalidOperationException or FormatException or OverflowException)
            {
                throw new VideoConversionException($"The decoder's info is not the expected JSON ({ex.Message}): {json.Trim()}", ex);
            }
        }

        private static JsonElement Member(JsonElement element, string name)
        {
            if (element.ValueKind != JsonValueKind.Object) throw new FormatException("an object was expected");
            foreach (JsonProperty property in element.EnumerateObject())
                if (property.Name.Equals(name, StringComparison.OrdinalIgnoreCase)) return property.Value;
            throw new FormatException($"'{name}' is missing");
        }

        private static int Number(JsonElement element, string name) => Member(element, name).GetInt32();

        private static int Positive(JsonElement element, string name) =>
            Number(element, name) is var value && value > 0 ? value : throw new FormatException($"'{name}' is {value}, not positive");
    }
}
