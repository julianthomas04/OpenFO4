#!/usr/bin/env bash
set -euo pipefail

work=$(cygpath -u "$OPENFO4_LIBREMPEG_WORK")
prefix="$work/prefix"
mkdir -p "$work/obj/x264" "$work/obj/librempeg" "$prefix"
export PKG_CONFIG_PATH="$prefix/lib/pkgconfig"

cd "$work/obj/x264"
"$work/sources/x264/configure" \
    --prefix="$prefix" --host=x86_64-w64-mingw32 \
    --enable-static --disable-cli --disable-opencl --disable-avs \
    --disable-swscale --disable-lavf --disable-ffms --disable-gpac \
    --disable-lsmash --disable-bashcompletion
make -j "$OPENFO4_LIBREMPEG_JOBS"
make install

cd "$work/obj/librempeg"
"$work/sources/librempeg/configure" \
    --prefix="$prefix" --arch=x86_64 --target-os=mingw32 \
    --disable-autodetect --disable-network --enable-agpl --disable-nonfree \
    --enable-gpl --enable-version3 --enable-static --disable-shared \
    --disable-everything --enable-ffmpeg --enable-ffprobe --disable-ffplay \
    --disable-doc --disable-debug --enable-libx264 \
    --pkg-config-flags=--static --extra-ldflags=-static \
    --enable-protocol=file,pipe --enable-demuxer=bink,mov,rawvideo,xwma,wav,flac \
    --enable-decoder=bink,bink2,binkaudio_dct,h264,rawvideo,wmav2,pcm_s16le,pcm_u8,pcm_s24le,pcm_s32le,pcm_f32le,adpcm_ms,adpcm_ima_wav,flac --enable-parser=h264,flac \
    --enable-encoder=libx264,aac,rawvideo,bmp,flac,pcm_s16le \
    --enable-muxer=mp4,rawvideo,image2,null,flac,wav \
    --enable-filter=scale,format,null,aformat,anull,acl2cl,ardftsrc,asf2sf,volumedetect
required_config=(
    CONFIG_AGPL CONFIG_GPL CONFIG_VERSION3 CONFIG_FFMPEG CONFIG_FFPROBE CONFIG_LIBX264_ENCODER
    CONFIG_BINK_DEMUXER CONFIG_MOV_DEMUXER CONFIG_BINK_DECODER CONFIG_BINK2_DECODER CONFIG_BINKAUDIO_DCT_DECODER CONFIG_AAC_ENCODER CONFIG_H264_DECODER
    CONFIG_RAWVIDEO_DEMUXER CONFIG_RAWVIDEO_DECODER CONFIG_RAWVIDEO_ENCODER CONFIG_RAWVIDEO_MUXER
    CONFIG_BMP_ENCODER CONFIG_IMAGE2_MUXER
    CONFIG_SCALE_FILTER CONFIG_FORMAT_FILTER CONFIG_H264_PARSER
    CONFIG_XWMA_DEMUXER CONFIG_WAV_DEMUXER CONFIG_WMAV2_DECODER CONFIG_FLAC_DEMUXER CONFIG_FLAC_DECODER CONFIG_FLAC_PARSER
    CONFIG_PCM_S16LE_DECODER CONFIG_PCM_U8_DECODER CONFIG_PCM_S24LE_DECODER
    CONFIG_PCM_S32LE_DECODER CONFIG_PCM_F32LE_DECODER CONFIG_ADPCM_MS_DECODER CONFIG_ADPCM_IMA_WAV_DECODER
    CONFIG_FLAC_ENCODER CONFIG_FLAC_MUXER CONFIG_PCM_S16LE_ENCODER CONFIG_WAV_MUXER CONFIG_MP4_MUXER CONFIG_NULL_MUXER
    CONFIG_AFORMAT_FILTER CONFIG_ANULL_FILTER CONFIG_ACL2CL_FILTER CONFIG_ARDFTSRC_FILTER
    CONFIG_ASF2SF_FILTER CONFIG_VOLUMEDETECT_FILTER CONFIG_FILE_PROTOCOL CONFIG_PIPE_PROTOCOL
)
for symbol in "${required_config[@]}"; do
    grep -qhE "^#define $symbol 1$" config.h config_components.h \
    || { echo "Required configuration missing: $symbol" >&2; exit 1; }
done
for forbidden in CONFIG_NONFREE CONFIG_NETWORK; do
    if grep -qE "^#define $forbidden 1$" config.h; then echo "Forbidden configuration: $forbidden" >&2; exit 1; fi
done
make -j "$OPENFO4_LIBREMPEG_JOBS"
make install

objdump -p "$prefix/bin/ffmpeg.exe" > "$work/ffmpeg-imports.txt"
objdump -p "$prefix/bin/ffprobe.exe" > "$work/ffprobe-imports.txt"
"$prefix/bin/ffmpeg.exe" -version > "$work/ffmpeg-version.txt"
"$prefix/bin/ffmpeg.exe" -buildconf > "$work/ffmpeg-buildconf.txt" 2>&1
"$prefix/bin/ffmpeg.exe" -decoders > "$work/ffmpeg-decoders.txt" 2>&1
"$prefix/bin/ffmpeg.exe" -encoders > "$work/ffmpeg-encoders.txt" 2>&1
