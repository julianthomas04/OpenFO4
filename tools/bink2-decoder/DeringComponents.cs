// SPDX-License-Identifier: GPL-3.0-or-later
namespace OpenFallout4.Bink2;

[Flags]
internal enum DeringComponents
{
    None = 0,
    Luma = 1,
    Chroma = 2,
    All = Luma | Chroma,
}
