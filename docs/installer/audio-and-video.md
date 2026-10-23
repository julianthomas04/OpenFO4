# Audio and video

The audio lane converts the game's sound and music to FLAC, and the engine finds
each sound by its original path in the archive. Conversion runs through ffmpeg
as a separate process. The binary is built from Librempeg, a fork of ffmpeg, by
`tools/librempeg`.

The video lane converts the game's Bink2 videos, a video format by RAD, to MP4.
Bink2 is decoded by a separate program, `tools/bink2-decoder`, which
runs as a child process. It exists because ffmpeg can't decode Bink2, and
keeping it separate also keeps its licence separate.
