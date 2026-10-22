// AV1 temporal units (../src/av1_obu.h): OBUs are found with and without
// size fields, broken ones are refused, and a key frame without a sequence
// header gets the last one in front of its first OBU after the temporal
// delimiter. No libwebrtc, no encoder.
//
// desktop/core/tests/mediasoup_loopback.rs builds and runs this with the
// host's C++ compiler. By hand, in desktop/native:
//   c++ -std=c++20 -O2 -I src -o av1_obu_test tests/av1_obu_test.cc && ./av1_obu_test
#include "av1_obu.h"

#include <cstdio>
#include <string>
#include <vector>

namespace av1 = gelabber::av1;

namespace
{
	int failures = 0;

	void Expect(bool holds, const std::string& what)
	{
		if (holds)
			return;
		++failures;
		std::printf("FAILED: %s\n", what.c_str());
	}

	using Bytes = std::vector<uint8_t>;

	Bytes Join(std::initializer_list<Bytes> parts)
	{
		Bytes out;
		for (const auto& part : parts)
			out.insert(out.end(), part.begin(), part.end());
		return out;
	}

	// An OBU of `type` with a size field and `payload` bytes of `fill`.
	Bytes Obu(uint8_t type, size_t payload, uint8_t fill)
	{
		Bytes out{ static_cast<uint8_t>((type << 3) | 0x02) };
		size_t length = payload;
		do
		{
			uint8_t byte = length & 0x7f;
			length >>= 7;
			if (length)
				byte |= 0x80;
			out.push_back(byte);
		} while (length);
		out.insert(out.end(), payload, fill);
		return out;
	}

	const Bytes kDelimiter = Obu(av1::kTemporalDelimiter, 0, 0);
	const Bytes kHeader    = Obu(av1::kSequenceHeader, 11, 0xa1);
	// OBU_FRAME (6), long enough for a two-byte size.
	const Bytes kFrame = Obu(6, 300, 0x5f);

	void FindsObus()
	{
		const Bytes unit = Join({ kDelimiter, kHeader, kFrame });
		const auto obus  = av1::Parse(unit.data(), unit.size());
		Expect(obus && obus->size() == 3, "three OBUs");
		if (!obus || obus->size() != 3)
			return;
		Expect((*obus)[0].type == av1::kTemporalDelimiter && (*obus)[0].end == kDelimiter.size(), "delimiter");
		Expect((*obus)[1].type == av1::kSequenceHeader && (*obus)[1].begin == kDelimiter.size(), "header");
		Expect((*obus)[2].type == 6 && (*obus)[2].end == unit.size(), "frame, two-byte size");
		Expect(av1::SequenceHeader(unit.data(), unit.size()) == kHeader, "the header, whole");

		// An extension byte, and a last OBU without a size field.
		const Bytes extended{ static_cast<uint8_t>((6 << 3) | 0x04 | 0x02), 0x08, 0x01, 0x77 };
		const Bytes unsized{ 6 << 3, 1, 2, 3, 4 };
		const Bytes mixed = Join({ extended, unsized });
		const auto more   = av1::Parse(mixed.data(), mixed.size());
		Expect(more && more->size() == 2 && (*more)[0].end == 4 && (*more)[1].end == mixed.size(), "extension, no size");
		Expect(av1::SequenceHeader(mixed.data(), mixed.size()).empty(), "no header");
	}

	void RefusesBrokenUnits()
	{
		const Bytes forbidden{ 0x80 | (6 << 3) | 0x02, 0 };
		Expect(!av1::Parse(forbidden.data(), forbidden.size()), "forbidden bit");
		Bytes cut = kFrame;
		cut.resize(cut.size() - 1);
		Expect(!av1::Parse(cut.data(), cut.size()), "OBU longer than the unit");
		const Bytes endless{ (6 << 3) | 0x02, 0x80, 0x80 };
		Expect(!av1::Parse(endless.data(), endless.size()), "size runs off the end");
		const Bytes tooLong{ (6 << 3) | 0x02, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01 };
		Expect(!av1::Parse(tooLong.data(), tooLong.size()), "size over eight bytes");
		const Bytes alone{ (6 << 3) | 0x04 };
		Expect(!av1::Parse(alone.data(), alone.size()), "extension byte missing");
		Expect(av1::Parse(nullptr, 0) && av1::Parse(nullptr, 0)->empty(), "empty unit");
	}

	void PutsTheHeaderInFront()
	{
		const Bytes key = Join({ kDelimiter, kFrame });
		Expect(
		  av1::WithSequenceHeader(key.data(), key.size(), kHeader) == Join({ kDelimiter, kHeader, kFrame }),
		  "after the delimiter");
		Expect(
		  av1::WithSequenceHeader(kFrame.data(), kFrame.size(), kHeader) == Join({ kHeader, kFrame }),
		  "without a delimiter: first");
		const Bytes complete = Join({ kDelimiter, kHeader, kFrame });
		Expect(
		  av1::WithSequenceHeader(complete.data(), complete.size(), kHeader) == complete, "one header only");
		Expect(av1::WithSequenceHeader(key.data(), key.size(), {}) == key, "no header known");
		const Bytes broken{ 0x80, 1, 2 };
		Expect(av1::WithSequenceHeader(broken.data(), broken.size(), kHeader) == broken, "broken unit as it is");
	}
} // namespace

int main()
{
	FindsObus();
	RefusesBrokenUnits();
	PutsTheHeaderInFront();
	if (failures > 0)
	{
		std::printf("%d check(s) failed\n", failures);
		return 1;
	}
	std::printf("AV1 OBUs: all checks passed\n");
	return 0;
}
