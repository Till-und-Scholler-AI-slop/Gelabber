// AV1 temporal units in the low overhead bitstream format (OBUs with size
// fields, AV1 spec section 5), as the system's encoders put them out and
// libwebrtc's RTP packetizer takes them. No libwebrtc types here, so that
// tests/av1_obu_test.cc builds without the core's libraries.
//
// A receiver can decode from a key frame only with a sequence header in
// front of it. Not every encoder repeats it there, so the core keeps the
// last one and puts it in front of a key frame that comes without one.
#ifndef GELABBER_AV1_OBU_H
#define GELABBER_AV1_OBU_H

#include <cstddef>
#include <cstdint>
#include <optional>
#include <vector>

namespace gelabber::av1
{
	constexpr uint8_t kSequenceHeader     = 1;
	constexpr uint8_t kTemporalDelimiter  = 2;

	struct Obu
	{
		uint8_t type;
		// The whole OBU, header included: [begin, end).
		size_t begin;
		size_t end;
	};

	// The OBUs of `data`, or nothing when it is no sequence of OBUs. An OBU
	// without a size field runs to the end, as only the last one may.
	inline std::optional<std::vector<Obu>> Parse(const uint8_t* data, size_t size)
	{
		std::vector<Obu> obus;
		size_t at = 0;
		while (at < size)
		{
			const uint8_t header = data[at];
			// forbidden_bit
			if (header & 0x80)
				return std::nullopt;
			const uint8_t type     = (header >> 3) & 0x0f;
			const bool extension   = header & 0x04;
			const bool sized       = header & 0x02;
			size_t payload         = at + 1 + (extension ? 1 : 0);
			if (payload > size)
				return std::nullopt;
			size_t end = size;
			if (sized)
			{
				// leb128, at most eight bytes.
				uint64_t length = 0;
				int bytes       = 0;
				for (;; ++bytes)
				{
					if (bytes == 8 || payload >= size)
						return std::nullopt;
					const uint8_t byte = data[payload++];
					length |= static_cast<uint64_t>(byte & 0x7f) << (7 * bytes);
					if (!(byte & 0x80))
						break;
				}
				if (length > size - payload)
					return std::nullopt;
				end = payload + static_cast<size_t>(length);
			}
			obus.push_back({ type, at, end });
			at = end;
		}
		return obus;
	}

	// The first sequence header OBU of `data`, whole; empty without one.
	inline std::vector<uint8_t> SequenceHeader(const uint8_t* data, size_t size)
	{
		const auto obus = Parse(data, size);
		if (obus)
			for (const auto& obu : *obus)
				if (obu.type == kSequenceHeader)
					return { data + obu.begin, data + obu.end };
		return {};
	}

	// `data` with `header` (a whole sequence header OBU) in front of its
	// first OBU that is no temporal delimiter. `data` as it is when it has a
	// sequence header already, when `header` is empty or when `data` is no
	// sequence of OBUs.
	inline std::vector<uint8_t> WithSequenceHeader(
	  const uint8_t* data, size_t size, const std::vector<uint8_t>& header)
	{
		std::vector<uint8_t> out(data, data + size);
		const auto obus = Parse(data, size);
		if (header.empty() || !obus)
			return out;
		size_t at = size;
		for (const auto& obu : *obus)
		{
			if (obu.type == kSequenceHeader)
				return out;
			if (obu.type != kTemporalDelimiter && at == size)
				at = obu.begin;
		}
		out.insert(out.begin() + static_cast<std::ptrdiff_t>(at), header.begin(), header.end());
		return out;
	}
} // namespace gelabber::av1

#endif
