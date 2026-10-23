using System.Numerics;

namespace AssetConverter.Havok
{
    /// <summary>
    /// Decodes hkaSplineCompressedAnimation blocks. Each block holds one 4-byte mask per transform track
    /// (quantization byte, then static/spline flags for position, rotation and scale), followed by the
    /// per-track channel data in track order. Blocks span maxFramesPerBlock frames and share their boundary frame.
    /// </summary>
    internal static class SplineAnimationDecoder
    {
        private const int MaskSize = 4;
        private const int AxisCount = 3;
        private const int AxisMask = 0x07;
        private const int StaticFlagsMask = 0x0F;
        private const int SplineFlagsShift = 4;

        /// <summary>Upper bound on frameCount x trackCount (decoded transforms held at once, ~40 bytes each); vanilla clips stay far below it.</summary>
        private const long MaxDecodedTransforms = 1L << 23;

        public static HavokTransform[][] Decode(HavokSplineAnimation animation)
        {
            Validate(animation);

            var data = new HavokBytes(animation.Data);
            var tracks = new HavokTransform[animation.TransformTrackCount][];
            for (int t = 0; t < tracks.Length; t++)
                tracks[t] = new HavokTransform[animation.FrameCount];

            int framesPerBlockStep = animation.MaxFramesPerBlock - 1;
            for (int block = 0; block < animation.BlockCount; block++)
            {
                int firstFrame = block * framesPerBlockStep;
                int lastFrame = block == animation.BlockCount - 1
                    ? animation.FrameCount - 1
                    : firstFrame + framesPerBlockStep - 1;
                if (firstFrame > lastFrame) continue;

                var channels = ReadBlock(animation, data, block);
                for (int t = 0; t < tracks.Length; t++)
                    for (int frame = firstFrame; frame <= lastFrame; frame++)
                        tracks[t][frame] = channels[t].Evaluate(frame - firstFrame);
            }
            return tracks;
        }

        private static void Validate(HavokSplineAnimation a)
        {
            if (a.FrameCount < 0 || a.TransformTrackCount < 0)
                throw new HavokFormatException($"Spline animation has {a.FrameCount} frames and {a.TransformTrackCount} tracks");
            long decodedTransforms = (long)a.FrameCount * a.TransformTrackCount;
            if (decodedTransforms > MaxDecodedTransforms)
                throw new HavokFormatException($"Spline animation of {a.FrameCount} frames x {a.TransformTrackCount} tracks exceeds the budget of {MaxDecodedTransforms} transforms");
            if (a.MaxFramesPerBlock < 2)
                throw new HavokFormatException($"Spline animation block size {a.MaxFramesPerBlock} is too small");
            if (a.FrameCount > 0 && a.BlockCount < 1)
                throw new HavokFormatException("Spline animation has frames but no blocks");
            long framesCovered = (long)a.BlockCount * (a.MaxFramesPerBlock - 1) + 1;
            if (a.FrameCount > framesCovered)
                throw new HavokFormatException($"{a.BlockCount} blocks of {a.MaxFramesPerBlock} frames cannot cover {a.FrameCount} frames");
            if (a.MaskAndQuantizationSize < a.TransformTrackCount * MaskSize)
                throw new HavokFormatException($"Mask size {a.MaskAndQuantizationSize} cannot hold {a.TransformTrackCount} track masks");
        }

        private static TrackChannels[] ReadBlock(HavokSplineAnimation animation, HavokBytes data, int block)
        {
            int blockStart = BlockOffset(animation, block);
            int blockEnd = block + 1 < animation.BlockOffsets.Count ? BlockOffset(animation, block + 1) : data.Length;
            data.Require(blockStart, animation.MaskAndQuantizationSize, $"block {block} masks");

            var reader = new SplineChannelReader(data, blockStart + animation.MaskAndQuantizationSize, blockStart);
            var channels = new TrackChannels[animation.TransformTrackCount];
            for (int t = 0; t < channels.Length; t++)
            {
                int mask = blockStart + t * MaskSize;
                channels[t] = reader.ReadTrack(data.U8(mask), data.U8(mask + 1), data.U8(mask + 2), data.U8(mask + 3));
            }

            if (reader.Position > blockEnd)
                throw new HavokFormatException($"Block {block} track data overruns into the next block ({reader.Position} > {blockEnd})");
            return channels;
        }

        private static int BlockOffset(HavokSplineAnimation animation, int block)
        {
            uint offset = animation.BlockOffsets[block];
            if (offset > int.MaxValue)
                throw new HavokFormatException($"Block {block} offset {offset} is beyond any addressable buffer");
            return (int)offset;
        }

        private sealed class SplineChannelReader
        {
            private readonly HavokBytes _bytes;
            private readonly int _origin;
            public int Position { get; private set; }

            public SplineChannelReader(HavokBytes bytes, int position, int origin)
            {
                _bytes = bytes;
                Position = position;
                _origin = origin;
            }

            public TrackChannels ReadTrack(byte quantization, byte positionFlags, byte rotationFlags, byte scaleFlags)
            {
                int vectorQuantPosition = quantization & 0x03;
                var rotationQuantization = HavokQuaternionCodec.Parse((quantization >> 2) & 0x0F);
                int vectorQuantScale = (quantization >> 6) & 0x03;

                var position = ReadVector(positionFlags, vectorQuantPosition, 0f);
                var rotation = ReadRotation(rotationFlags, rotationQuantization);
                var scale = ReadVector(scaleFlags, vectorQuantScale, 1f);
                return new TrackChannels(position, rotation, scale);
            }

            private VectorChannel ReadVector(byte flags, int quantization, float defaultValue)
            {
                int splineAxes = (flags >> SplineFlagsShift) & AxisMask;
                int staticAxes = flags & StaticFlagsMask & AxisMask & ~splineAxes;
                var baseValue = new Vector3(defaultValue);
                if (staticAxes == 0 && splineAxes == 0)
                    return new VectorChannel(baseValue, 0, null);

                int bits = VectorBits(quantization);
                (int degree, byte[] knots) = splineAxes != 0 ? ReadSplineHeader() : (0, Array.Empty<byte>());
                Align(4);

                var min = new float[AxisCount];
                var max = new float[AxisCount];
                for (int axis = 0; axis < AxisCount; axis++)
                {
                    if (HasAxis(splineAxes, axis))
                    {
                        min[axis] = ReadFloat();
                        max[axis] = ReadFloat();
                    }
                    else if (HasAxis(staticAxes, axis))
                    {
                        baseValue = SetAxis(baseValue, axis, ReadFloat());
                    }
                }

                HavokSplineCurve? curve = null;
                if (splineAxes != 0)
                {
                    int dimensions = System.Numerics.BitOperations.PopCount((uint)splineAxes);
                    int pointCount = SplinePointCount(knots, degree);
                    var points = new float[pointCount * dimensions];
                    float maxRaw = (1 << bits) - 1;
                    int next = 0;
                    for (int p = 0; p < pointCount; p++)
                    {
                        for (int axis = 0; axis < AxisCount; axis++)
                        {
                            if (!HasAxis(splineAxes, axis)) continue;
                            float raw = bits == 8 ? ReadU8() : ReadU16();
                            points[next++] = min[axis] + (max[axis] - min[axis]) * (raw / maxRaw);
                        }
                    }
                    curve = new HavokSplineCurve(degree, knots, points, dimensions);
                }

                Align(4);
                return new VectorChannel(baseValue, splineAxes, curve);
            }

            private static bool HasAxis(int axes, int axis) => (axes >> axis & 1) != 0;

            private static int VectorBits(int quantization) => quantization switch
            {
                0 => 8,
                1 => 16,
                _ => throw new HavokUnsupportedQuantizationException($"Position/scale quantization {quantization} is not defined"),
            };

            private RotationChannel ReadRotation(byte flags, HavokRotationQuantization quantization)
            {
                bool isStatic = (flags & StaticFlagsMask) != 0;
                bool isSpline = (flags >> SplineFlagsShift) != 0;
                if (!isStatic && !isSpline)
                    return new RotationChannel(Quaternion.Identity, null);

                HavokQuaternionCodec.RequireSupported(quantization);
                int size = HavokQuaternionCodec.ByteSize(quantization);
                int alignment = HavokQuaternionCodec.Alignment(quantization);

                if (isSpline)
                {
                    (int degree, byte[] knots) = ReadSplineHeader();
                    Align(alignment);
                    int pointCount = SplinePointCount(knots, degree);
                    var points = new float[pointCount * 4];
                    for (int p = 0; p < pointCount; p++)
                    {
                        var q = HavokQuaternionCodec.Decode(quantization, _bytes, Position);
                        Position += size;
                        points[p * 4] = q.X; points[p * 4 + 1] = q.Y; points[p * 4 + 2] = q.Z; points[p * 4 + 3] = q.W;
                    }
                    Align(4);
                    return new RotationChannel(Quaternion.Identity, new HavokSplineCurve(degree, knots, points, 4));
                }

                Align(alignment);
                var staticValue = HavokQuaternionCodec.Decode(quantization, _bytes, Position);
                Position += size;
                Align(4);
                return new RotationChannel(staticValue, null);
            }

            private static int SplinePointCount(byte[] knots, int degree)
            {
                int pointCount = knots.Length - degree - 1;
                if (pointCount < 1)
                    throw new HavokFormatException($"Spline with {knots.Length} knots and degree {degree} has no control points");
                return pointCount;
            }

            private (int degree, byte[] knots) ReadSplineHeader()
            {
                int itemCount = ReadU16() + 1;
                int degree = ReadU8();
                int knotCount = itemCount + degree + 1;
                var knots = _bytes.Copy(Position, knotCount, "spline knots");
                Position += knotCount;
                return (degree, knots);
            }

            private void Align(int alignment)
            {
                int offset = Position - _origin;
                Position = _origin + (offset + alignment - 1) / alignment * alignment;
            }

            private int ReadU8() { int v = _bytes.U8(Position); Position += 1; return v; }
            private int ReadU16() { int v = _bytes.U16(Position); Position += 2; return v; }
            private float ReadFloat() { float v = _bytes.F32(Position); Position += 4; return v; }
        }

        private static Vector3 SetAxis(Vector3 v, int axis, float value)
        {
            switch (axis)
            {
                case 0: v.X = value; break;
                case 1: v.Y = value; break;
                default: v.Z = value; break;
            }
            return v;
        }

        private sealed record VectorChannel(Vector3 Base, int SplineAxes, HavokSplineCurve? Curve)
        {
            public Vector3 Evaluate(float u)
            {
                if (Curve is null) return Base;
                Span<float> values = stackalloc float[AxisCount];
                Curve.Evaluate(u, values);
                var result = Base;
                int next = 0;
                for (int axis = 0; axis < AxisCount; axis++)
                    if ((SplineAxes >> axis & 1) != 0)
                        result = SetAxis(result, axis, values[next++]);
                return result;
            }
        }

        private sealed record RotationChannel(Quaternion Static, HavokSplineCurve? Curve)
        {
            public Quaternion Evaluate(float u)
            {
                if (Curve is null) return Static;
                Span<float> q = stackalloc float[4];
                Curve.Evaluate(u, q);
                return Quaternion.Normalize(new Quaternion(q[0], q[1], q[2], q[3]));
            }
        }

        private sealed record TrackChannels(VectorChannel Position, RotationChannel Rotation, VectorChannel Scale)
        {
            public HavokTransform Evaluate(float u) => new(Position.Evaluate(u), Rotation.Evaluate(u), Scale.Evaluate(u));
        }
    }
}
