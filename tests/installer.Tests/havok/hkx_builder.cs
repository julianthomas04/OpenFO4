using System.Numerics;
using System.Text;

namespace InstallerTests.Havok
{
    /// <summary>
    /// Writes Havok 2014.1 64-bit packfiles in the Havok 2014.1 layout: 0x40-byte global header, 0x40-byte
    /// section headers (<c>__classnames__</c>, <c>__data__</c>), local / global / virtual fixup tables after the section data.
    /// Offsets handed out by this class are relative to the start of the data section.
    /// </summary>
    internal sealed class HkxBuilder
    {
        public const int GlobalHeaderSize = 0x40;
        public const int SectionHeaderSize = 0x40;
        public const int PointerSize = 8;

        private readonly List<byte> _data = new();
        private readonly List<(int Src, int Dst)> _localFixups = new();
        private readonly List<(int Object, string ClassName)> _objects = new();

        public string ContentsVersion { get; set; } = "hk_2014.1.0-r1";

        public int FileVersion { get; set; } = 11;

        public byte PointerBytes { get; set; } = 8;

        public byte LittleEndian { get; set; } = 1;

        public int FixupTerminatorPadding { get; set; }

        public int NewObject(string className, int size)
        {
            int offset = Alloc(size);
            _objects.Add((offset, className));
            return offset;
        }

        public int Alloc(int size)
        {
            while (_data.Count % 16 != 0) _data.Add(0);
            int offset = _data.Count;
            _data.AddRange(new byte[size]);
            return offset;
        }

        public int AddString(string text)
        {
            int offset = Alloc(text.Length + 1);
            for (int i = 0; i < text.Length; i++) _data[offset + i] = (byte)text[i];
            return offset;
        }

        public void Ptr(int field, int target) => _localFixups.Add((field, target));

        public void StringField(int field, string text) => Ptr(field, AddString(text));

        public void ObjectPtr(int field, int obj) => Ptr(field, obj);

        public int Array(int field, int elementSize, int count)
        {
            U32(field + PointerSize, (uint)count);
            U32(field + PointerSize + 4, (uint)count | 0x80000000u);
            if (count == 0) return 0;
            int payload = Alloc(elementSize * count);
            Ptr(field, payload);
            return payload;
        }

        public void U8(int offset, byte value) => _data[offset] = value;

        public void U16(int offset, ushort value) => Write(offset, BitConverter.GetBytes(value));

        public void I16(int offset, short value) => Write(offset, BitConverter.GetBytes(value));

        public void U32(int offset, uint value) => Write(offset, BitConverter.GetBytes(value));

        public void I32(int offset, int value) => Write(offset, BitConverter.GetBytes(value));

        public void F32(int offset, float value) => Write(offset, BitConverter.GetBytes(value));

        public void Vec3(int offset, Vector3 v)
        {
            F32(offset, v.X);
            F32(offset + 4, v.Y);
            F32(offset + 8, v.Z);
        }

        public void Quat(int offset, Quaternion q)
        {
            F32(offset, q.X);
            F32(offset + 4, q.Y);
            F32(offset + 8, q.Z);
            F32(offset + 12, q.W);
        }

        /// <summary>QsTransform: translation, pad, rotation, scale, pad (48 bytes).</summary>
        public void QsTransform(int offset, Vector3 translation, Quaternion rotation, Vector3 scale)
        {
            Vec3(offset, translation);
            Quat(offset + 16, rotation);
            Vec3(offset + 32, scale);
        }

        public byte[] Build()
        {
            var classNames = new List<byte>();
            var nameOffsets = new Dictionary<string, int>();
            foreach (string name in _objects.Select(o => o.ClassName).Distinct())
            {
                classNames.AddRange([0x11, 0x22, 0x33, 0x44, 0x09]);
                nameOffsets[name] = classNames.Count;
                classNames.AddRange(Encoding.ASCII.GetBytes(name));
                classNames.Add(0);
            }
            while (classNames.Count % 16 != 0) classNames.Add(0xFF);

            while (_data.Count % 16 != 0) _data.Add(0);
            int dataLength = _data.Count;
            int localSize = (_localFixups.Count + FixupTerminatorPadding) * 8;
            int virtualSize = (_objects.Count + FixupTerminatorPadding) * 12;
            int local = dataLength;
            int global = local + localSize;
            int virt = global;
            int exports = virt + virtualSize;

            int classNamesStart = GlobalHeaderSize + 2 * SectionHeaderSize;
            int dataStart = classNamesStart + classNames.Count;

            var file = new List<byte>();
            file.AddRange([0x57, 0xE0, 0xE0, 0x57, 0x10, 0xC0, 0xC0, 0x10]);
            file.AddRange(new byte[GlobalHeaderSize - file.Count]);
            Patch(file, 0x0C, BitConverter.GetBytes(FileVersion));
            file[0x10] = PointerBytes;
            file[0x11] = LittleEndian;
            Patch(file, 0x14, BitConverter.GetBytes(2));
            for (int i = 0; i < ContentsVersion.Length && i < 16; i++) file[0x28 + i] = (byte)ContentsVersion[i];

            AddSectionHeader(file, "__classnames__", classNamesStart, classNames.Count, classNames.Count, classNames.Count, classNames.Count);
            AddSectionHeader(file, "__data__", dataStart, local, global, virt, exports);

            file.AddRange(classNames);
            file.AddRange(_data);
            foreach (var (src, dst) in _localFixups)
            {
                file.AddRange(BitConverter.GetBytes(src));
                file.AddRange(BitConverter.GetBytes(dst));
            }
            AddTerminators(file, FixupTerminatorPadding, 8);
            foreach (var (obj, className) in _objects)
            {
                file.AddRange(BitConverter.GetBytes(obj));
                file.AddRange(BitConverter.GetBytes(0));
                file.AddRange(BitConverter.GetBytes(nameOffsets[className]));
            }
            AddTerminators(file, FixupTerminatorPadding, 12);
            return file.ToArray();
        }

        private static void AddTerminators(List<byte> file, int count, int entrySize)
        {
            for (int i = 0; i < count; i++)
                file.AddRange(Enumerable.Repeat((byte)0xFF, entrySize));
        }

        private static void AddSectionHeader(List<byte> file, string name, int start, int local, int global, int virt, int exports)
        {
            var header = new byte[SectionHeaderSize];
            Encoding.ASCII.GetBytes(name).CopyTo(header, 0);
            BitConverter.GetBytes(start).CopyTo(header, 0x14);
            BitConverter.GetBytes(local).CopyTo(header, 0x18);
            BitConverter.GetBytes(global).CopyTo(header, 0x1C);
            BitConverter.GetBytes(virt).CopyTo(header, 0x20);
            BitConverter.GetBytes(exports).CopyTo(header, 0x24);
            file.AddRange(header);
        }

        private static void Patch(List<byte> file, int offset, byte[] bytes)
        {
            for (int i = 0; i < bytes.Length; i++) file[offset + i] = bytes[i];
        }

        private void Write(int offset, byte[] bytes)
        {
            for (int i = 0; i < bytes.Length; i++) _data[offset + i] = bytes[i];
        }
    }

    internal sealed record HkBone(string Name, short Parent, Vector3 Translation, Quaternion Rotation, Vector3 Scale);

    internal sealed record HkAnnotationSpec(string TrackName, (float Time, string Text)[] Annotations);

    /// <summary>Common fields every animation object carries (duration, track count, annotation tracks).</summary>
    internal abstract record HkAnimationSpec(float Duration, int Tracks, HkAnnotationSpec[]? Annotations = null);

    internal sealed record HkInterleavedSpec(float Duration, int Tracks, int Frames, HkAnnotationSpec[]? Annotations = null)
        : HkAnimationSpec(Duration, Tracks, Annotations);

    internal sealed record HkSplineSpec(float Duration, int Tracks, int Frames, int Blocks, int MaxFramesPerBlock, int MaskSize,
        float FrameDuration, byte[][] BlockData, HkAnnotationSpec[]? Annotations = null)
        : HkAnimationSpec(Duration, Tracks, Annotations);

    internal sealed record HkLosslessSpec(float Duration, int Tracks, int Frames, float[] DynamicTranslations, float[] StaticTranslations,
        ushort[] TranslationDescriptors, Quaternion[] DynamicRotations, Quaternion[] StaticRotations, ushort[] RotationDescriptors,
        int FloatTracks = 0) : HkAnimationSpec(Duration, Tracks);

    internal sealed record HkBindingSpec(string? OriginalSkeleton, short[] TrackToBone, byte BlendHint, HkAnimationSpec Animation);

    /// <summary>Data-section offsets of the objects <see cref="HkaFile.Build"/> wrote, for tests that corrupt individual fields.</summary>
    internal sealed record HkaLayout(IReadOnlyList<int> Skeletons, IReadOnlyList<int> Bindings, IReadOnlyList<int> Animations);

    /// <summary>Assembles an hkRootLevelContainer / hkaAnimationContainer packfile out of skeletons and bindings.</summary>
    internal static class HkaFile
    {
        public static byte[] Build(IReadOnlyList<(string Name, HkBone[] Bones)> skeletons, IReadOnlyList<HkBindingSpec> bindings,
            Action<HkxBuilder, HkaLayout>? tweak = null)
        {
            var b = new HkxBuilder();
            int root = b.NewObject("hkRootLevelContainer", 0x10);
            int variants = b.Array(root, 0x18, 1);
            b.StringField(variants, "Animation Data");
            b.StringField(variants + 0x08, "hkaAnimationContainer");
            int container = b.NewObject("hkaAnimationContainer", 0x48);
            b.ObjectPtr(variants + 0x10, container);

            var skeletonOffsets = new List<int>();
            var bindingOffsets = new List<int>();
            int skeletonArray = b.Array(container + 0x10, HkxBuilder.PointerSize, skeletons.Count);
            for (int i = 0; i < skeletons.Count; i++)
            {
                skeletonOffsets.Add(WriteSkeleton(b, skeletons[i].Name, skeletons[i].Bones));
                b.ObjectPtr(skeletonArray + i * HkxBuilder.PointerSize, skeletonOffsets[i]);
            }

            int bindingArray = b.Array(container + 0x30, HkxBuilder.PointerSize, bindings.Count);
            var animations = new Dictionary<HkAnimationSpec, int>();
            for (int i = 0; i < bindings.Count; i++)
            {
                if (!animations.TryGetValue(bindings[i].Animation, out int animation))
                    animations[bindings[i].Animation] = animation = WriteAnimation(b, bindings[i].Animation);
                bindingOffsets.Add(WriteBinding(b, bindings[i], animation));
                b.ObjectPtr(bindingArray + i * HkxBuilder.PointerSize, bindingOffsets[i]);
            }

            tweak?.Invoke(b, new HkaLayout(skeletonOffsets, bindingOffsets, animations.Values.ToList()));
            return b.Build();
        }

        /// <summary>A packfile whose root container holds no animation container (a behavior graph, character data).</summary>
        public static byte[] BuildWithoutAnimationContainer()
        {
            var b = new HkxBuilder();
            int root = b.NewObject("hkRootLevelContainer", 0x10);
            int variants = b.Array(root, 0x18, 1);
            b.StringField(variants, "Behavior");
            b.StringField(variants + 0x08, "hkbBehaviorGraph");
            b.ObjectPtr(variants + 0x10, b.NewObject("hkbBehaviorGraph", 0x20));
            return b.Build();
        }

        private static int WriteSkeleton(HkxBuilder b, string name, HkBone[] bones)
        {
            int skeleton = b.NewObject("hkaSkeleton", 0x60);
            b.StringField(skeleton + 0x10, name);
            int parents = b.Array(skeleton + 0x18, 2, bones.Length);
            int boneArray = b.Array(skeleton + 0x28, 16, bones.Length);
            int pose = b.Array(skeleton + 0x38, 48, bones.Length);
            for (int i = 0; i < bones.Length; i++)
            {
                b.I16(parents + i * 2, bones[i].Parent);
                b.StringField(boneArray + i * 16, bones[i].Name);
                b.QsTransform(pose + i * 48, bones[i].Translation, bones[i].Rotation, bones[i].Scale);
            }
            return skeleton;
        }

        private static int WriteBinding(HkxBuilder b, HkBindingSpec spec, int animation)
        {
            int binding = b.NewObject("hkaAnimationBinding", 0x58);
            if (spec.OriginalSkeleton is not null) b.StringField(binding + 0x10, spec.OriginalSkeleton);
            b.ObjectPtr(binding + 0x18, animation);
            int map = b.Array(binding + 0x20, 2, spec.TrackToBone.Length);
            for (int i = 0; i < spec.TrackToBone.Length; i++) b.I16(map + i * 2, spec.TrackToBone[i]);
            b.U8(binding + 0x50, spec.BlendHint);
            return binding;
        }

        private static int WriteAnimation(HkxBuilder b, HkAnimationSpec spec) => spec switch
        {
            HkInterleavedSpec s => WriteInterleaved(b, s),
            HkSplineSpec s => WriteSpline(b, s),
            HkLosslessSpec s => WriteLossless(b, s),
            _ => throw new ArgumentOutOfRangeException(nameof(spec)),
        };

        private static int WriteInterleaved(HkxBuilder b, HkInterleavedSpec s)
        {
            int obj = b.NewObject("hkaInterleavedUncompressedAnimation", 0x50);
            WriteCommon(b, obj, s);
            int transforms = b.Array(obj + 0x38, 48, s.Tracks * s.Frames);
            for (int frame = 0; frame < s.Frames; frame++)
                for (int track = 0; track < s.Tracks; track++)
                    b.QsTransform(transforms + (frame * s.Tracks + track) * 48,
                        new Vector3(frame, track, 0f), Quaternion.Identity, Vector3.One);
            return obj;
        }

        private static int WriteSpline(HkxBuilder b, HkSplineSpec s)
        {
            int obj = b.NewObject("hkaSplineCompressedAnimation", 0xA8);
            WriteCommon(b, obj, s);
            b.I32(obj + 0x38, s.Frames);
            b.I32(obj + 0x3C, s.Blocks);
            b.I32(obj + 0x40, s.MaxFramesPerBlock);
            b.I32(obj + 0x44, s.MaskSize);
            b.F32(obj + 0x50, s.FrameDuration);
            int offsets = b.Array(obj + 0x58, 4, s.Blocks);
            int cursor = 0;
            for (int i = 0; i < s.Blocks; i++)
            {
                b.U32(offsets + i * 4, (uint)cursor);
                cursor += s.BlockData[i].Length;
            }
            int data = b.Array(obj + 0x98, 1, cursor);
            cursor = 0;
            foreach (var block in s.BlockData)
                for (int i = 0; i < block.Length; i++) b.U8(data + cursor++, block[i]);
            return obj;
        }

        private static int WriteLossless(HkxBuilder b, HkLosslessSpec s)
        {
            int obj = b.NewObject("hkaLosslessCompressedAnimation", 0xE0);
            WriteCommon(b, obj, s);
            b.I32(obj + 0x1C, s.FloatTracks);
            b.I32(obj + 0xD8, s.Frames);
            WriteFloats(b, obj + 0x38, s.DynamicTranslations);
            WriteFloats(b, obj + 0x48, s.StaticTranslations);
            WriteU16(b, obj + 0x58, s.TranslationDescriptors, 8);
            WriteQuats(b, obj + 0x68, s.DynamicRotations);
            WriteQuats(b, obj + 0x78, s.StaticRotations);
            WriteU16(b, obj + 0x88, s.RotationDescriptors, 2);
            b.Array(obj + 0x98, 4, 0);
            b.Array(obj + 0xA8, 4, 0);
            WriteU16(b, obj + 0xB8, new ushort[s.Tracks * 4], 8);
            return obj;
        }

        private static void WriteCommon(HkxBuilder b, int obj, HkAnimationSpec s)
        {
            b.F32(obj + 0x14, s.Duration);
            b.I32(obj + 0x18, s.Tracks);
            var tracks = s.Annotations ?? [];
            int array = b.Array(obj + 0x28, 0x18, tracks.Length);
            for (int t = 0; t < tracks.Length; t++)
            {
                int track = array + t * 0x18;
                b.StringField(track, tracks[t].TrackName);
                int annotations = b.Array(track + 0x08, 16, tracks[t].Annotations.Length);
                for (int a = 0; a < tracks[t].Annotations.Length; a++)
                {
                    b.F32(annotations + a * 16, tracks[t].Annotations[a].Time);
                    b.StringField(annotations + a * 16 + 8, tracks[t].Annotations[a].Text);
                }
            }
        }

        private static void WriteFloats(HkxBuilder b, int field, float[] values)
        {
            int payload = b.Array(field, 4, values.Length);
            for (int i = 0; i < values.Length; i++) b.F32(payload + i * 4, values[i]);
        }

        private static void WriteQuats(HkxBuilder b, int field, Quaternion[] values)
        {
            int payload = b.Array(field, 16, values.Length);
            for (int i = 0; i < values.Length; i++) b.Quat(payload + i * 16, values[i]);
        }

        private static void WriteU16(HkxBuilder b, int field, ushort[] values, int elementSize)
        {
            int perElement = elementSize / 2;
            int payload = b.Array(field, elementSize, values.Length / perElement);
            for (int i = 0; i < values.Length; i++) b.U16(payload + i * 2, values[i]);
        }
    }
}
