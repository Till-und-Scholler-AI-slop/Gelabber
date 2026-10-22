// The audio device module's devices as the web client sees them: listing
// with stable ids, lookup and selection. Header-only, used by the engine
// (gelabber_media.cc) and the audio pipeline (audio_pipeline.h).
#ifndef GELABBER_AUDIO_DEVICES_H
#define GELABBER_AUDIO_DEVICES_H

#include <api/audio/audio_device.h>

#include <cstdint>
#include <map>
#include <optional>
#include <string>
#include <vector>

namespace gelabber
{
	// Stands for the system default where the module has no index for it.
	inline constexpr int kDefaultAudioDevice = -1;
#if defined(WEBRTC_WIN)
	// Core Audio lists the endpoints only; the default is a role that is
	// selected on its own (SelectRecordingDevice, SelectPlayoutDevice).
	inline constexpr bool kDefaultAudioDeviceIsListed = false;
#else
	// Index 0 is the system default (PulseAudio).
	inline constexpr bool kDefaultAudioDeviceIsListed = true;
#endif

	struct AudioDeviceEntry
	{
		// The module's index, or kDefaultAudioDevice.
		int index;
		std::string id;
		std::string name;
	};

	// Devices in module order, the system default first with id "". Other
	// ids are the module's GUID where it has one (Windows endpoint ids);
	// libwebrtc's PulseAudio module reports none, so there the display
	// name is the id, numbered when names repeat.
	template<typename Count, typename NameOf>
	std::vector<AudioDeviceEntry> ListAudioDevices(Count count, NameOf nameOf)
	{
		std::vector<AudioDeviceEntry> out;
		if (!kDefaultAudioDeviceIsListed)
			out.push_back({ kDefaultAudioDevice, "", "Default" });
		std::map<std::string, int> seen;
		char name[webrtc::kAdmMaxDeviceNameSize];
		char guid[webrtc::kAdmMaxGuidSize];
		for (int i = 0, n = count(); i < n; ++i)
		{
			name[0] = guid[0] = '\0';
			if (nameOf(static_cast<uint16_t>(i), name, guid) != 0)
				continue;
			std::string id;
			if (i > 0 || !kDefaultAudioDeviceIsListed)
			{
				id = guid[0] != '\0' ? guid : name;
				if (const int repeat = ++seen[id]; repeat > 1)
					id += " (" + std::to_string(repeat) + ")";
			}
			out.push_back({ i, id, name });
		}
		return out;
	}

	inline std::vector<AudioDeviceEntry> RecordingDevices(webrtc::AudioDeviceModule& adm)
	{
		return ListAudioDevices(
		  [&] { return adm.RecordingDevices(); },
		  [&](uint16_t i, char* name, char* guid) { return adm.RecordingDeviceName(i, name, guid); });
	}

	inline std::vector<AudioDeviceEntry> PlayoutDevices(webrtc::AudioDeviceModule& adm)
	{
		return ListAudioDevices(
		  [&] { return adm.PlayoutDevices(); },
		  [&](uint16_t i, char* name, char* guid) { return adm.PlayoutDeviceName(i, name, guid); });
	}

	// The index of the device with this id, if there is one.
	inline std::optional<int> FindAudioDevice(const std::vector<AudioDeviceEntry>& devices, const std::string& id)
	{
		for (const auto& device : devices)
			if (device.id == id)
				return device.index;
		return std::nullopt;
	}

	// The device to go back to when a switch away from `id` fails: that one,
	// or the system default once it is gone.
	inline std::optional<int> PreviousAudioDevice(const std::vector<AudioDeviceEntry>& devices, const std::string& id)
	{
		if (const auto index = FindAudioDevice(devices, id))
			return index;
		return FindAudioDevice(devices, "");
	}

	// Windows has two defaults. The system default is the console role:
	// streams on the communications role make Windows turn every other
	// application down for as long as they run.
	inline int32_t SelectRecordingDevice(webrtc::AudioDeviceModule& adm, int index)
	{
		return index == kDefaultAudioDevice ? adm.SetRecordingDevice(webrtc::AudioDeviceModule::kDefaultDevice)
		                                    : adm.SetRecordingDevice(static_cast<uint16_t>(index));
	}

	inline int32_t SelectPlayoutDevice(webrtc::AudioDeviceModule& adm, int index)
	{
		return index == kDefaultAudioDevice ? adm.SetPlayoutDevice(webrtc::AudioDeviceModule::kDefaultDevice)
		                                    : adm.SetPlayoutDevice(static_cast<uint16_t>(index));
	}
} // namespace gelabber

#endif
