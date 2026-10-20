// Switching an audio device (../src/audio_device_switch.h) on a fake device
// module: a running switch restarts on the new device, a device that cannot
// be selected or started gives the previous one back, running again, and a
// side nobody wants running is only selected. No libwebrtc, no sound card.
//
// desktop/core/tests/voice.rs builds and runs this with the host's C++
// compiler. By hand, in desktop/native:
//   c++ -std=c++20 -O2 -I src -o switch_test tests/audio_device_switch_test.cc && ./switch_test
#include "audio_device_switch.h"

#include <cstdio>
#include <set>
#include <string>
#include <vector>

using gelabber::DeviceOps;
using gelabber::SwitchDevice;
using gelabber::SwitchResult;

namespace
{
	int failures = 0;

	void Expect(bool holds, const std::string& what)
	{
		if (holds)
			return;
		std::printf("FAILED: %s\n", what.c_str());
		++failures;
	}

	// One side of a device module as libwebrtc's behave: a device is
	// selected only while the side is not initialized, a refused selection
	// leaves none selected (Core Audio releases the endpoint first), and
	// only a selected device initializes.
	class FakeDevice
	{
	public:
		static constexpr int kNone = -100;

		explicit FakeDevice(int selected, bool running = false)
		  : selected(selected), isInitialized(running), running(running)
		{
		}

		DeviceOps Ops()
		{
			return {
				[this] {
					calls.push_back("stop");
					isInitialized = running = false;
					return 0;
				},
				[this] { return isInitialized; },
				[this](int device) {
					calls.push_back("select " + std::to_string(device));
					if (isInitialized)
						return -1;
					if (refuseSelect.count(device) > 0)
					{
						selected = kNone;
						return -1;
					}
					selected = device;
					return 0;
				},
				[this] {
					calls.push_back("init");
					if (selected == kNone)
						return -1;
					isInitialized = true;
					return 0;
				},
				[this] {
					calls.push_back("start");
					if (!isInitialized || refuseStart.count(selected) > 0)
						return -1;
					running = true;
					return 0;
				},
			};
		}

		std::string Calls() const
		{
			std::string out;
			for (const auto& call : calls)
				out += (out.empty() ? "" : ", ") + call;
			return out;
		}

		std::set<int> refuseSelect;
		std::set<int> refuseStart;
		int selected;
		bool isInitialized;
		bool running;
		std::vector<std::string> calls;
	};

	void RunningSwitchSucceeds()
	{
		FakeDevice device(1, true);
		const auto result = SwitchDevice(device.Ops(), true, 2, 1);
		Expect(result == SwitchResult::Switched, "running switch: switched");
		Expect(device.selected == 2 && device.running, "running switch: runs on the new device");
		Expect(
		  device.Calls() == "stop, select 2, init, start", "running switch: stop, select, start: " + device.Calls());
	}

	void RefusedSelectKeepsThePrevious()
	{
		FakeDevice device(1, true);
		device.refuseSelect = { 2 };
		const auto result = SwitchDevice(device.Ops(), true, 2, 1);
		Expect(result == SwitchResult::KeptPrevious, "refused select: kept the previous device");
		Expect(device.selected == 1 && device.running, "refused select: runs on the previous device again");
		Expect(
		  device.Calls() == "stop, select 2, select 1, init, start",
		  "refused select: the previous device restarts: " + device.Calls());
	}

	void RefusedStartKeepsThePrevious()
	{
		FakeDevice device(1, true);
		device.refuseStart = { 2 };
		const auto result = SwitchDevice(device.Ops(), true, 2, 1);
		Expect(result == SwitchResult::KeptPrevious, "refused start: kept the previous device");
		Expect(device.selected == 1 && device.running, "refused start: runs on the previous device again");
		// Stopped before the previous device can be selected again.
		Expect(
		  device.Calls() == "stop, select 2, init, start, stop, select 1, init, start",
		  "refused start: stop, then the previous device restarts: " + device.Calls());
	}

	void FailedRollbackStops()
	{
		FakeDevice device(1, true);
		device.refuseStart = { 1, 2 };
		const auto result = SwitchDevice(device.Ops(), true, 2, 1);
		Expect(result == SwitchResult::Stopped, "failed rollback: stopped");
		Expect(!device.running && !device.isInitialized, "failed rollback: neither runs nor stays initialized");

		FakeDevice gone(1, true);
		gone.refuseSelect = { 2 };
		Expect(
		  SwitchDevice(gone.Ops(), true, 2, std::nullopt) == SwitchResult::Stopped,
		  "no previous device: stopped");
		Expect(!gone.running, "no previous device: nothing runs");
	}

	// After a restart that failed, the module is stopped while its users
	// still want it running: the next switch starts it, also to the device
	// that is selected already.
	void WantedButStoppedStarts()
	{
		FakeDevice device(1);
		const auto result = SwitchDevice(device.Ops(), true, 0, 1);
		Expect(result == SwitchResult::Switched, "wanted but stopped: switched");
		Expect(device.selected == 0 && device.running, "wanted but stopped: runs on the new device");

		FakeDevice same(1);
		Expect(SwitchDevice(same.Ops(), true, 1, 1) == SwitchResult::Switched, "the same device: switched");
		Expect(same.running, "the same device: runs again");
	}

	void InitializedButNotWantedOnlySelects()
	{
		FakeDevice device(1);
		device.isInitialized = true;
		const auto result = SwitchDevice(device.Ops(), false, 2, 1);
		Expect(result == SwitchResult::Switched, "initialized, not wanted: switched");
		Expect(device.selected == 2 && !device.running, "initialized, not wanted: selected, not started");
		Expect(device.Calls() == "stop, select 2", "initialized, not wanted: stop, select: " + device.Calls());
	}

	void NotWantedOnlySelects()
	{
		FakeDevice device(1);
		const auto result = SwitchDevice(device.Ops(), false, 2, 1);
		Expect(result == SwitchResult::Switched, "not wanted: switched");
		Expect(device.selected == 2 && !device.running, "not wanted: selected, not started");
		Expect(device.Calls() == "select 2", "not wanted: only the selection: " + device.Calls());

		FakeDevice refused(1);
		refused.refuseSelect = { 2 };
		Expect(
		  SwitchDevice(refused.Ops(), false, 2, 1) == SwitchResult::KeptPrevious,
		  "not wanted, refused select: kept the previous device");
		Expect(refused.selected == 1 && !refused.running, "not wanted, refused select: selected again, not started");
	}
} // namespace

int main()
{
	RunningSwitchSucceeds();
	RefusedSelectKeepsThePrevious();
	RefusedStartKeepsThePrevious();
	FailedRollbackStops();
	WantedButStoppedStarts();
	InitializedButNotWantedOnlySelects();
	NotWantedOnlySelects();
	if (failures > 0)
	{
		std::printf("%d check(s) failed\n", failures);
		return 1;
	}
	std::printf("audio device switch: all checks passed\n");
	return 0;
}
