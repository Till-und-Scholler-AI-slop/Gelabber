// Where the samples of a filled PipeWire buffer are (app_audio_pipewire.cc),
// kept free of the sound server so that it also runs in a plain test
// (../tests/pipewire_chunk_test.cc).
//
// A buffer maps `maxsize` bytes; its chunk says where in them the samples
// start and how many bytes they take. Read as they are, an offset with a
// large size leads past the mapping, on PipeWire's thread. So both are kept
// inside it, as pw-cat reads them: the offset at most `maxsize`, the size at
// most what is left behind it (GEL-47).
#ifndef GELABBER_PIPEWIRE_CHUNK_H
#define GELABBER_PIPEWIRE_CHUNK_H

#include <algorithm>
#include <cstddef>
#include <cstdint>

namespace gelabber
{
	// Bytes from the start of the buffer's data, and whole frames from there.
	struct ChunkSpan
	{
		size_t offset;
		size_t frames;
	};

	inline ChunkSpan ChunkFrames(uint32_t maxsize, uint32_t offset, uint32_t size, size_t frameBytes)
	{
		const uint32_t start = std::min(offset, maxsize);
		return { start, std::min(size, maxsize - start) / frameBytes };
	}
} // namespace gelabber

#endif
