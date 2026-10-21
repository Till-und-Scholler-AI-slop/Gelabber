// Switching the microphone or the speakers while they run (issue #296), kept
// free of libwebrtc so that it also runs in a plain test
// (../tests/audio_device_switch_test.cc).
//
// A device module selects a device only while that side is stopped and not
// initialized, so a switch stops it, selects and starts it again. Whether it
// starts again is what its users want, not what the module says: after a
// restart that failed, the module is stopped while the voice pipeline still
// thinks it records and never starts it itself; the next switch, even to
// the same device, has to. When the new device cannot be selected or does
// not start, the previous one is selected and started again instead of
// leaving the side silent.
#ifndef GELABBER_AUDIO_DEVICE_SWITCH_H
#define GELABBER_AUDIO_DEVICE_SWITCH_H

#include <functional>
#include <optional>

namespace gelabber
{
	// One side of a device module, recording or playout. The int results
	// are the module's: 0 on success.
	struct DeviceOps
	{
		std::function<int()> stop;
		std::function<bool()> initialized;
		std::function<int(int)> select;
		std::function<int()> init;
		std::function<int()> start;
	};

	enum class SwitchResult
	{
		// The new device is selected, and runs if wanted.
		Switched,
		// The new device could not be used; the previous one is selected
		// again, and runs if wanted.
		KeptPrevious,
		// Neither could be used: the side is stopped.
		Stopped,
	};

	// Selects the device `index` and runs it if `wanted`; `previous` is the
	// device selected before, if it is still there.
	inline SwitchResult SwitchDevice(const DeviceOps& ops, bool wanted, int index, std::optional<int> previous)
	{
		// Selects `device` and starts it if wanted; stopped again if it
		// does not start.
		const auto use = [&](int device) {
			if (ops.select(device) != 0)
				return false;
			if (!wanted || (ops.init() == 0 && ops.start() == 0))
				return true;
			ops.stop();
			return false;
		};
		if (wanted || ops.initialized())
			ops.stop();
		if (use(index))
			return SwitchResult::Switched;
		if (previous && use(*previous))
			return SwitchResult::KeptPrevious;
		return SwitchResult::Stopped;
	}
} // namespace gelabber

#endif
