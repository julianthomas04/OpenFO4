// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Julian Thomas and OpenFO4 contributors

using System.Buffers.Binary;
using System.Globalization;
using System.IO;
using SharpBSABA2.BA2Util;
using Xunit;

namespace InstallerTests.BA2Sharp;

public class DdsFormatTests
{
    [Fact]
    public void ConstantsAndEnumsMatchTheIndependentFormatContract()
    {
        Assert.Equal(542327876, DDS.DDS_MAGIC);
        Assert.Equal(4, DDS.DDS_FOURCC);
        Assert.Equal(64, DDS.DDS_RGB);
        Assert.Equal(65, DDS.DDS_RGBA);
        Assert.Equal(4103, DDS.DDS_HEADER_FLAGS_TEXTURE);
        Assert.Equal(131072, DDS.DDS_HEADER_FLAGS_MIPMAP);
        Assert.Equal(524288, DDS.DDS_HEADER_FLAGS_LINEARSIZE);
        Assert.Equal(4096, DDS.DDS_SURFACE_FLAGS_TEXTURE);
        Assert.Equal(4194312, DDS.DDS_SURFACE_FLAGS_MIPMAP);
        Assert.Equal(0, DDS.DDS_ALPHA_MODE_UNKNOWN);
        Assert.Equal(4u, DDS.DDS_RESOURCE_MISC_TEXTURECUBE);

        AssertEnumValues<DXGI_FORMAT>(
        ("R8G8B8A8_UNORM", 28UL), ("R8G8B8A8_UNORM_SRGB", 29UL), ("R8_UNORM", 61UL), ("BC1_UNORM", 71UL),
        ("BC1_UNORM_SRGB", 72UL), ("BC2_UNORM", 74UL), ("BC3_UNORM", 77UL), ("BC3_UNORM_SRGB", 78UL),
        ("BC4_UNORM", 80UL), ("BC5_UNORM", 83UL), ("BC5_SNORM", 84UL), ("B5G6R5_UNORM", 85UL),
        ("B8G8R8A8_UNORM", 87UL), ("B8G8R8X8_UNORM", 88UL), ("BC6H_UF16", 95UL), ("BC7_UNORM", 98UL),
        ("BC7_UNORM_SRGB", 99UL)
        );
        AssertEnumValues<DXGI_FORMAT_FULL>(
        ("UNKNOWN", 0UL), ("R32G32B32A32_TYPELESS", 1UL), ("R32G32B32A32_FLOAT", 2UL), ("R32G32B32A32_UINT", 3UL),
        ("R32G32B32A32_SINT", 4UL), ("R32G32B32_TYPELESS", 5UL), ("R32G32B32_FLOAT", 6UL), ("R32G32B32_UINT", 7UL),
        ("R32G32B32_SINT", 8UL), ("R16G16B16A16_TYPELESS", 9UL), ("R16G16B16A16_FLOAT", 10UL), ("R16G16B16A16_UNORM", 11UL),
        ("R16G16B16A16_UINT", 12UL), ("R16G16B16A16_SNORM", 13UL), ("R16G16B16A16_SINT", 14UL), ("R32G32_TYPELESS", 15UL),
        ("R32G32_FLOAT", 16UL), ("R32G32_UINT", 17UL), ("R32G32_SINT", 18UL), ("R32G8X24_TYPELESS", 19UL),
        ("D32_FLOAT_S8X24_UINT", 20UL), ("R32_FLOAT_X8X24_TYPELESS", 21UL), ("X32_TYPELESS_G8X24_UINT", 22UL), ("R10G10B10A2_TYPELESS", 23UL),
        ("R10G10B10A2_UNORM", 24UL), ("R10G10B10A2_UINT", 25UL), ("R11G11B10_FLOAT", 26UL), ("R8G8B8A8_TYPELESS", 27UL),
        ("R8G8B8A8_UNORM", 28UL), ("R8G8B8A8_UNORM_SRGB", 29UL), ("R8G8B8A8_UINT", 30UL), ("R8G8B8A8_SNORM", 31UL),
        ("R8G8B8A8_SINT", 32UL), ("R16G16_TYPELESS", 33UL), ("R16G16_FLOAT", 34UL), ("R16G16_UNORM", 35UL),
        ("R16G16_UINT", 36UL), ("R16G16_SNORM", 37UL), ("R16G16_SINT", 38UL), ("R32_TYPELESS", 39UL),
        ("D32_FLOAT", 40UL), ("R32_FLOAT", 41UL), ("R32_UINT", 42UL), ("R32_SINT", 43UL),
        ("R24G8_TYPELESS", 44UL), ("D24_UNORM_S8_UINT", 45UL), ("R24_UNORM_X8_TYPELESS", 46UL), ("X24_TYPELESS_G8_UINT", 47UL),
        ("R8G8_TYPELESS", 48UL), ("R8G8_UNORM", 49UL), ("R8G8_UINT", 50UL), ("R8G8_SNORM", 51UL),
        ("R8G8_SINT", 52UL), ("R16_TYPELESS", 53UL), ("R16_FLOAT", 54UL), ("D16_UNORM", 55UL),
        ("R16_UNORM", 56UL), ("R16_UINT", 57UL), ("R16_SNORM", 58UL), ("R16_SINT", 59UL),
        ("R8_TYPELESS", 60UL), ("R8_UNORM", 61UL), ("R8_UINT", 62UL), ("R8_SNORM", 63UL),
        ("R8_SINT", 64UL), ("A8_UNORM", 65UL), ("R1_UNORM", 66UL), ("R9G9B9E5_SHAREDEXP", 67UL),
        ("R8G8_B8G8_UNORM", 68UL), ("G8R8_G8B8_UNORM", 69UL), ("BC1_TYPELESS", 70UL), ("BC1_UNORM", 71UL),
        ("BC1_UNORM_SRGB", 72UL), ("BC2_TYPELESS", 73UL), ("BC2_UNORM", 74UL), ("BC2_UNORM_SRGB", 75UL),
        ("BC3_TYPELESS", 76UL), ("BC3_UNORM", 77UL), ("BC3_UNORM_SRGB", 78UL), ("BC4_TYPELESS", 79UL),
        ("BC4_UNORM", 80UL), ("BC4_SNORM", 81UL), ("BC5_TYPELESS", 82UL), ("BC5_UNORM", 83UL),
        ("BC5_SNORM", 84UL), ("B5G6R5_UNORM", 85UL), ("B5G5R5A1_UNORM", 86UL), ("B8G8R8A8_UNORM", 87UL),
        ("B8G8R8X8_UNORM", 88UL), ("R10G10B10_XR_BIAS_A2_UNORM", 89UL), ("B8G8R8A8_TYPELESS", 90UL), ("B8G8R8A8_UNORM_SRGB", 91UL),
        ("B8G8R8X8_TYPELESS", 92UL), ("B8G8R8X8_UNORM_SRGB", 93UL), ("BC6H_TYPELESS", 94UL), ("BC6H_UF16", 95UL),
        ("BC6H_SF16", 96UL), ("BC7_TYPELESS", 97UL), ("BC7_UNORM", 98UL), ("BC7_UNORM_SRGB", 99UL),
        ("AYUV", 100UL), ("Y410", 101UL), ("Y416", 102UL), ("NV12", 103UL),
        ("P010", 104UL), ("P016", 105UL), ("_420_OPAQUE", 106UL), ("YUY2", 107UL),
        ("Y210", 108UL), ("Y216", 109UL), ("NV11", 110UL), ("AI44", 111UL),
        ("IA44", 112UL), ("P8", 113UL), ("A8P8", 114UL), ("B4G4R4A4_UNORM", 115UL),
        ("P208", 130UL), ("V208", 131UL), ("V408", 132UL), ("FORCE_UINT", 4294967295UL)
        );
        AssertEnumValues<DDSCAPS2>(
        ("CUBEMAP", 512UL), ("CUBEMAP_POSITIVEX", 1024UL), ("CUBEMAP_NEGATIVEX", 2048UL), ("CUBEMAP_POSITIVEY", 4096UL),
        ("CUBEMAP_NEGATIVEY", 8192UL), ("CUBEMAP_POSITIVEZ", 16384UL), ("CUBEMAP_NEGATIVEZ", 32768UL), ("CUBEMAP_ALLFACES", 64512UL)
        );
        AssertEnumValues<DXT10_RESOURCE_DIMENSION>(
        ("DIMENSION_TEXTURE1D", 2UL), ("DIMENSION_TEXTURE2D", 3UL), ("DIMENSION_TEXTURE3D", 4UL)
        );
    }

    [Fact]
    public void EnumFlagsAttributesMatchTheContract()
    {
        Assert.False(typeof(DXGI_FORMAT).IsDefined(typeof(FlagsAttribute), inherit: false));
        Assert.False(typeof(DXGI_FORMAT_FULL).IsDefined(typeof(FlagsAttribute), inherit: false));
        Assert.True(typeof(DDSCAPS2).IsDefined(typeof(FlagsAttribute), inherit: false));
        Assert.False(typeof(DXT10_RESOURCE_DIMENSION).IsDefined(typeof(FlagsAttribute), inherit: false));
    }

    [Fact]
    public void FourCcUsesLowBytesAndPlacesThemInLittleEndianOrder()
    {
        Assert.Equal(0x31545844u, DDS.MAKEFOURCC('D', 'X', 'T', '1'));
        Assert.Equal(0x30315844u, DDS.MAKEFOURCC('D', 'X', '1', '0'));
        Assert.Equal(0xFFBC2300u, checked(DDS.MAKEFOURCC('\u0100', '\u0123', '\u1ABC', '\uFFFF')));
    }

    [Fact]
    public void LegacyHeaderWritesTheExact124ByteWireLayoutAndKeepsWriterOpen()
    {
        Assert.Equal(124u, DDS_HEADER.GetSize());
        Assert.Equal(32u, DDS_PIXELFORMAT.GetSize());

        var header = new DDS_HEADER
        {
            dwSize = 0x01020304,
            dwHeaderFlags = 0x11121314,
            dwHeight = 0x21222324,
            dwWidth = 0x31323334,
            dwPitchOrLinearSize = 0x41424344,
            dwDepth = 0x51525354,
            dwMipMapCount = 0x61626364,
            dwReserved1 = 0x71727374,
            PixelFormat = new DDS_PIXELFORMAT(0x81828384, 0x91929394, 0xA1A2A3A4, 0xB1B2B3B4,
                0xC1C2C3C4, 0xD1D2D3D4, 0xE1E2E3E4, 0xF1F2F3F4),
            dwSurfaceFlags = 0x02030405,
            dwCubemapFlags = 0x12131415,
            dwReserved2 = 0x22232425
        };

        uint[] words =
        [
            0x01020304, 0x11121314, 0x21222324, 0x31323334, 0x41424344, 0x51525354, 0x61626364,
            0x71727374, 0x71727374, 0x71727374, 0x71727374, 0x71727374, 0x71727374, 0x71727374,
            0x71727374, 0x71727374, 0x71727374, 0x71727374,
            0x81828384, 0x91929394, 0xA1A2A3A4, 0xB1B2B3B4, 0xC1C2C3C4, 0xD1D2D3D4, 0xE1E2E3E4, 0xF1F2F3F4,
            0x02030405, 0x12131415, 0x22232425, 0x22232425, 0x22232425
        ];
        byte[] prefix = [0xAA, 0xBB, 0xCC];
        using var stream = new MemoryStream();
        stream.Write(prefix);
        stream.Position = prefix.Length;
        using var writer = new BinaryWriter(stream, System.Text.Encoding.UTF8, leaveOpen: true);

        header.Write(writer);
        Assert.Equal((long)(prefix.Length + 124), stream.Position);
        writer.Write((byte)0xA5);

        var expected = new byte[prefix.Length + 125];
        prefix.CopyTo(expected, 0);
        WriteWordsLittleEndian(words, expected.AsSpan(prefix.Length, 124));
        expected[^1] = 0xA5;
        Assert.Equal(expected, stream.ToArray());
        Assert.True(stream.CanWrite);
    }

    [Fact]
    public void Dxt10HeaderWritesFiveUnsignedFieldsInWireOrder()
    {
        Assert.Equal(20u, DDS_HEADER_DXT10.GetSize());
        var header = new DDS_HEADER_DXT10
        {
            dxgiFormat = 0x01020304,
            resourceDimension = 0x11121314,
            miscFlag = 0x21222324,
            arraySize = 0x31323334,
            miscFlags2 = 0x41424344
        };
        byte[] prefix = [0xAA, 0xBB, 0xCC];
        using var stream = new MemoryStream();
        stream.Write(prefix);
        stream.Position = prefix.Length;
        using var writer = new BinaryWriter(stream, System.Text.Encoding.UTF8, leaveOpen: true);

        header.Write(writer);
        Assert.Equal((long)(prefix.Length + 20), stream.Position);
        writer.Write((byte)0xA5);

        Assert.Equal(new byte[]
        {
            0xAA, 0xBB, 0xCC,
            0x04, 0x03, 0x02, 0x01, 0x14, 0x13, 0x12, 0x11,
            0x24, 0x23, 0x22, 0x21, 0x34, 0x33, 0x32, 0x31,
            0x44, 0x43, 0x42, 0x41, 0xA5
        }, stream.ToArray());
        Assert.True(stream.CanWrite);
    }

    [Fact]
    public void HeaderWritePropagatesOutputErrorsAndLeavesWriterUsable()
    {
        using var stream = new FailOnceWriteStream();
        using var writer = new BinaryWriter(stream, System.Text.Encoding.UTF8, leaveOpen: true);
        var header = new DDS_HEADER_DXT10 { dxgiFormat = 1 };

        var exception = Assert.Throws<IOException>(() => header.Write(writer));

        Assert.Same(stream.Failure, exception);
        writer.Write((byte)0xA5);
        Assert.Equal(new byte[] { 0xA5 }, stream.ToArray());
        Assert.True(stream.CanWrite);
    }

    [Fact]
    public void PixelFormatConstructorAssignsFieldsPositionally()
    {
        var format = new DDS_PIXELFORMAT(1, 2, 3, 4, 5, 6, 7, 8);

        Assert.Equal(new uint[] { 1, 2, 3, 4, 5, 6, 7, 8 },
            new[] { format.dwSize, format.dwFlags, format.dwFourCC, format.dwRGBBitCount,
                format.dwRBitMask, format.dwGBitMask, format.dwBBitMask, format.dwABitMask });
    }

    private static void AssertEnumValues<TEnum>(params (string Name, ulong Value)[] expected)
        where TEnum : struct, Enum
    {
        var names = Enum.GetNames<TEnum>();
        Assert.Equal(expected.Select(item => item.Name).OrderBy(name => name),
            names.OrderBy(name => name));
        foreach (var item in expected)
            Assert.Equal(item.Value, Convert.ToUInt64(Enum.Parse<TEnum>(item.Name), CultureInfo.InvariantCulture));
    }

    private static void WriteWordsLittleEndian(ReadOnlySpan<uint> words, Span<byte> destination)
    {
        for (var i = 0; i < words.Length; i++)
            BinaryPrimitives.WriteUInt32LittleEndian(destination.Slice(i * 4, 4), words[i]);
    }

    private sealed class FailOnceWriteStream : MemoryStream
    {
        private bool _shouldFail = true;

        public IOException Failure { get; } = new("Synthetic output failure.");

        public override void Write(byte[] buffer, int offset, int count)
        {
            ThrowOnFirstWrite();
            base.Write(buffer, offset, count);
        }

        public override void Write(ReadOnlySpan<byte> buffer)
        {
            ThrowOnFirstWrite();
            base.Write(buffer);
        }

        private void ThrowOnFirstWrite()
        {
            if (!_shouldFail)
                return;

            _shouldFail = false;
            throw Failure;
        }
    }
}
