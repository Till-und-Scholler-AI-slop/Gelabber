// The part of a PipeWire buffer that application sound is read from
// (../src/pipewire_chunk.h, GEL-47): whole frames from the chunk's offset,
// never past the mapped memory, whatever offset and size the chunk carries.
// No sound server.
//
// desktop/core/tests/voice.rs builds and runs this with the host's C++
// compiler. By hand, in desktop/native:
//   c++ -std=c++20 -O2 -I src -o chunk_test tests/pipewire_chunk_test.cc && ./chunk_test
#include "pipewire_chunk.h"

#include <cstdint>
#include <cstdio>
#include <string>

using gelabber::ChunkFrames;

namespace
{
	// 32-bit float stereo, as the capture streams negotiate it.
	constexpr size_t kFrameBytes = 2 * sizeof(float);

	int failures = 0;

	void Expect(bool holds, const std::string& what)
	{
		if (holds)
			return;
		std::printf("FAILED: %s\n", what.c_str());
		++failures;
	}
} // namespace

int main()
{
	{
		const auto span = ChunkFrames(4096, 0, 4096, kFrameBytes);
		Expect(span.offset == 0 && span.frames == 512, "a full buffer: all of it");
	}
	{
		const auto span = ChunkFrames(4096, 0, 1024, kFrameBytes);
		Expect(span.offset == 0 && span.frames == 128, "a chunk shorter than the buffer: the chunk");
	}
	{
		const auto span = ChunkFrames(4096, 0, 8192, kFrameBytes);
		Expect(span.frames == 512, "a size beyond the buffer: up to its end");
	}
	{
		// The case of the ticket: the old reading took min(size, maxsize)
		// from the offset on, 1024 bytes past the end.
		const auto span = ChunkFrames(4096, 1024, 4096, kFrameBytes);
		Expect(span.offset == 1024 && span.frames == 384, "an offset with a large size: up to the buffer's end");
	}
	{
		const auto span = ChunkFrames(4096, 1024, 1024, kFrameBytes);
		Expect(span.offset == 1024 && span.frames == 128, "an offset with a size that fits: the chunk");
	}
	{
		const auto span = ChunkFrames(4096, 8192, 4096, kFrameBytes);
		Expect(span.offset <= 4096 && span.frames == 0, "an offset past the buffer: nothing");
	}
	{
		const auto span = ChunkFrames(4096, 4096, 4096, kFrameBytes);
		Expect(span.frames == 0, "an offset at the buffer's end: nothing");
	}
	{
		const auto span = ChunkFrames(4096, 0, 4100, kFrameBytes);
		Expect(span.frames == 512, "a partial frame at the end is left out");
	}
	{
		const auto span = ChunkFrames(4096, 4092, 4096, kFrameBytes);
		Expect(span.frames == 0, "less than a frame left: nothing");
	}
	{
		const auto span = ChunkFrames(UINT32_MAX, UINT32_MAX, UINT32_MAX, kFrameBytes);
		Expect(span.frames == 0, "the largest values do not wrap around");
	}
	// Whatever a chunk says, what is read stays inside the buffer.
	for (uint32_t maxsize : { 0u, 7u, 8u, 960u, 4096u, 3840u })
		for (uint32_t offset : { 0u, 1u, 4u, 8u, 100u, 960u, 3839u, 3840u, 4096u, 5000u, UINT32_MAX })
			for (uint32_t size : { 0u, 1u, 8u, 960u, 3840u, 4096u, 9000u, UINT32_MAX })
			{
				const auto span = ChunkFrames(maxsize, offset, size, kFrameBytes);
				const uint64_t end = static_cast<uint64_t>(span.offset) + span.frames * kFrameBytes;
				Expect(end <= maxsize && span.frames * kFrameBytes <= size,
				  "inside the buffer and the chunk: maxsize " + std::to_string(maxsize) + ", offset " +
				    std::to_string(offset) + ", size " + std::to_string(size));
			}
	if (failures)
	{
		std::printf("%d failed\n", failures);
		return 1;
	}
	std::printf("pipewire chunk: all passed\n");
	return 0;
}
