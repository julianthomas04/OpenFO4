// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Julian Thomas and OpenFO4 contributors
// Format facts: https://learn.microsoft.com/en-us/windows/win32/direct3ddds/dds-header and https://learn.microsoft.com/en-us/windows/win32/direct3ddds/dds-pixelformat; DXGI values: https://github.com/microsoft/win32metadata/blob/a7da6031bba126ace3ab3641b8e1291966960e87/generation/WinSDK/RecompiledIdlHeaders/shared/dxgiformat.h.

using System.IO;
using System.Runtime.InteropServices;

namespace SharpBSABA2.BA2Util
{
    public static class DDS
    {
        public const int DDS_MAGIC = 542327876;
        public const int DDS_FOURCC = 4;
        public const int DDS_RGB = 64;
        public const int DDS_RGBA = 65;
        public const int DDS_HEADER_FLAGS_TEXTURE = 4103;
        public const int DDS_HEADER_FLAGS_MIPMAP = 131072;
        public const int DDS_HEADER_FLAGS_LINEARSIZE = 524288;
        public const int DDS_SURFACE_FLAGS_TEXTURE = 4096;
        public const int DDS_SURFACE_FLAGS_MIPMAP = 4194312;
        public const int DDS_ALPHA_MODE_UNKNOWN = 0;
        public const uint DDS_RESOURCE_MISC_TEXTURECUBE = 4;

        public static uint MAKEFOURCC(char ch0, char ch1, char ch2, char ch3) =>
            ((uint)ch0 & 0xFFu)
            | (((uint)ch1 & 0xFFu) << 8)
            | (((uint)ch2 & 0xFFu) << 16)
            | (((uint)ch3 & 0xFFu) << 24);
    }
    public enum DXGI_FORMAT : int
    {
        R8G8B8A8_UNORM = 28,
        R8G8B8A8_UNORM_SRGB = 29,
        R8_UNORM = 61,
        BC1_UNORM = 71,
        BC1_UNORM_SRGB = 72,
        BC2_UNORM = 74,
        BC3_UNORM = 77,
        BC3_UNORM_SRGB = 78,
        BC4_UNORM = 80,
        BC5_UNORM = 83,
        BC5_SNORM = 84,
        B5G6R5_UNORM = 85,
        B8G8R8A8_UNORM = 87,
        B8G8R8X8_UNORM = 88,
        BC6H_UF16 = 95,
        BC7_UNORM = 98,
        BC7_UNORM_SRGB = 99,
    }

    public enum DXGI_FORMAT_FULL : uint
    {
        UNKNOWN = 0u,
        R32G32B32A32_TYPELESS = 1u,
        R32G32B32A32_FLOAT = 2u,
        R32G32B32A32_UINT = 3u,
        R32G32B32A32_SINT = 4u,
        R32G32B32_TYPELESS = 5u,
        R32G32B32_FLOAT = 6u,
        R32G32B32_UINT = 7u,
        R32G32B32_SINT = 8u,
        R16G16B16A16_TYPELESS = 9u,
        R16G16B16A16_FLOAT = 10u,
        R16G16B16A16_UNORM = 11u,
        R16G16B16A16_UINT = 12u,
        R16G16B16A16_SNORM = 13u,
        R16G16B16A16_SINT = 14u,
        R32G32_TYPELESS = 15u,
        R32G32_FLOAT = 16u,
        R32G32_UINT = 17u,
        R32G32_SINT = 18u,
        R32G8X24_TYPELESS = 19u,
        D32_FLOAT_S8X24_UINT = 20u,
        R32_FLOAT_X8X24_TYPELESS = 21u,
        X32_TYPELESS_G8X24_UINT = 22u,
        R10G10B10A2_TYPELESS = 23u,
        R10G10B10A2_UNORM = 24u,
        R10G10B10A2_UINT = 25u,
        R11G11B10_FLOAT = 26u,
        R8G8B8A8_TYPELESS = 27u,
        R8G8B8A8_UNORM = 28u,
        R8G8B8A8_UNORM_SRGB = 29u,
        R8G8B8A8_UINT = 30u,
        R8G8B8A8_SNORM = 31u,
        R8G8B8A8_SINT = 32u,
        R16G16_TYPELESS = 33u,
        R16G16_FLOAT = 34u,
        R16G16_UNORM = 35u,
        R16G16_UINT = 36u,
        R16G16_SNORM = 37u,
        R16G16_SINT = 38u,
        R32_TYPELESS = 39u,
        D32_FLOAT = 40u,
        R32_FLOAT = 41u,
        R32_UINT = 42u,
        R32_SINT = 43u,
        R24G8_TYPELESS = 44u,
        D24_UNORM_S8_UINT = 45u,
        R24_UNORM_X8_TYPELESS = 46u,
        X24_TYPELESS_G8_UINT = 47u,
        R8G8_TYPELESS = 48u,
        R8G8_UNORM = 49u,
        R8G8_UINT = 50u,
        R8G8_SNORM = 51u,
        R8G8_SINT = 52u,
        R16_TYPELESS = 53u,
        R16_FLOAT = 54u,
        D16_UNORM = 55u,
        R16_UNORM = 56u,
        R16_UINT = 57u,
        R16_SNORM = 58u,
        R16_SINT = 59u,
        R8_TYPELESS = 60u,
        R8_UNORM = 61u,
        R8_UINT = 62u,
        R8_SNORM = 63u,
        R8_SINT = 64u,
        A8_UNORM = 65u,
        R1_UNORM = 66u,
        R9G9B9E5_SHAREDEXP = 67u,
        R8G8_B8G8_UNORM = 68u,
        G8R8_G8B8_UNORM = 69u,
        BC1_TYPELESS = 70u,
        BC1_UNORM = 71u,
        BC1_UNORM_SRGB = 72u,
        BC2_TYPELESS = 73u,
        BC2_UNORM = 74u,
        BC2_UNORM_SRGB = 75u,
        BC3_TYPELESS = 76u,
        BC3_UNORM = 77u,
        BC3_UNORM_SRGB = 78u,
        BC4_TYPELESS = 79u,
        BC4_UNORM = 80u,
        BC4_SNORM = 81u,
        BC5_TYPELESS = 82u,
        BC5_UNORM = 83u,
        BC5_SNORM = 84u,
        B5G6R5_UNORM = 85u,
        B5G5R5A1_UNORM = 86u,
        B8G8R8A8_UNORM = 87u,
        B8G8R8X8_UNORM = 88u,
        R10G10B10_XR_BIAS_A2_UNORM = 89u,
        B8G8R8A8_TYPELESS = 90u,
        B8G8R8A8_UNORM_SRGB = 91u,
        B8G8R8X8_TYPELESS = 92u,
        B8G8R8X8_UNORM_SRGB = 93u,
        BC6H_TYPELESS = 94u,
        BC6H_UF16 = 95u,
        BC6H_SF16 = 96u,
        BC7_TYPELESS = 97u,
        BC7_UNORM = 98u,
        BC7_UNORM_SRGB = 99u,
        AYUV = 100u,
        Y410 = 101u,
        Y416 = 102u,
        NV12 = 103u,
        P010 = 104u,
        P016 = 105u,
        _420_OPAQUE = 106u,
        YUY2 = 107u,
        Y210 = 108u,
        Y216 = 109u,
        NV11 = 110u,
        AI44 = 111u,
        IA44 = 112u,
        P8 = 113u,
        A8P8 = 114u,
        B4G4R4A4_UNORM = 115u,
        P208 = 130u,
        V208 = 131u,
        V408 = 132u,
        FORCE_UINT = 4294967295u,
    }

    [System.Flags]
    public enum DDSCAPS2 : uint
    {
        CUBEMAP = 512u,
        CUBEMAP_POSITIVEX = 1024u,
        CUBEMAP_NEGATIVEX = 2048u,
        CUBEMAP_POSITIVEY = 4096u,
        CUBEMAP_NEGATIVEY = 8192u,
        CUBEMAP_POSITIVEZ = 16384u,
        CUBEMAP_NEGATIVEZ = 32768u,
        CUBEMAP_ALLFACES = 64512u,
    }

    public enum DXT10_RESOURCE_DIMENSION : int
    {
        DIMENSION_TEXTURE1D = 2,
        DIMENSION_TEXTURE2D = 3,
        DIMENSION_TEXTURE3D = 4,
    }
    [StructLayout(LayoutKind.Sequential, Pack = 1)]
    public struct DDS_HEADER
    {
        public uint dwSize;
        public uint dwHeaderFlags;
        public uint dwHeight;
        public uint dwWidth;
        public uint dwPitchOrLinearSize;
        public uint dwDepth;
        public uint dwMipMapCount;
        // Compatibility wrapper scalar repeated into the 11 legacy reserved DWORD slots by Write. Current callers use zero.
        public uint dwReserved1;
        public DDS_PIXELFORMAT PixelFormat;
        public uint dwSurfaceFlags;
        public uint dwCubemapFlags;
        // Compatibility wrapper scalar repeated into the 3 final reserved DWORD slots by Write. Current callers use zero.
        public uint dwReserved2;

        public static uint GetSize() => 124;

        public void Write(BinaryWriter bw)
        {
            bw.Write(dwSize);
            bw.Write(dwHeaderFlags);
            bw.Write(dwHeight);
            bw.Write(dwWidth);
            bw.Write(dwPitchOrLinearSize);
            bw.Write(dwDepth);
            bw.Write(dwMipMapCount);

            for (var i = 0; i < 11; i++)
                bw.Write(dwReserved1);

            bw.Write(PixelFormat.dwSize);
            bw.Write(PixelFormat.dwFlags);
            bw.Write(PixelFormat.dwFourCC);
            bw.Write(PixelFormat.dwRGBBitCount);
            bw.Write(PixelFormat.dwRBitMask);
            bw.Write(PixelFormat.dwGBitMask);
            bw.Write(PixelFormat.dwBBitMask);
            bw.Write(PixelFormat.dwABitMask);
            bw.Write(dwSurfaceFlags);
            bw.Write(dwCubemapFlags);

            for (var i = 0; i < 3; i++)
                bw.Write(dwReserved2);
        }
    }

    [StructLayout(LayoutKind.Sequential, Pack = 1)]
    public struct DDS_HEADER_DXT10
    {
        public uint dxgiFormat;
        public uint resourceDimension;
        public uint miscFlag;
        public uint arraySize;
        public uint miscFlags2;

        public static uint GetSize() => 20;

        public void Write(BinaryWriter bw)
        {
            bw.Write(dxgiFormat);
            bw.Write(resourceDimension);
            bw.Write(miscFlag);
            bw.Write(arraySize);
            bw.Write(miscFlags2);
        }
    }

    [StructLayout(LayoutKind.Sequential, Pack = 1)]
    public struct DDS_PIXELFORMAT
    {
        public uint dwSize;
        public uint dwFlags;
        public uint dwFourCC;
        public uint dwRGBBitCount;
        public uint dwRBitMask;
        public uint dwGBitMask;
        public uint dwBBitMask;
        public uint dwABitMask;

        public DDS_PIXELFORMAT(uint size, uint flags, uint fourCC, uint rgbBitCount, uint rBitMask, uint gBitMask, uint bBitMask, uint aBitMask)
        {
            dwSize = size;
            dwFlags = flags;
            dwFourCC = fourCC;
            dwRGBBitCount = rgbBitCount;
            dwRBitMask = rBitMask;
            dwGBitMask = gBitMask;
            dwBBitMask = bBitMask;
            dwABitMask = aBitMask;
        }

        public static uint GetSize() => 32;
    }
}
