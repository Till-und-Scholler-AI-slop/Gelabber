#include "mf_video.h"

#include "av1_obu.h"

#include <api/video/encoded_image.h>
#include <api/video/i420_buffer.h>
#include <api/video/video_bitrate_allocation.h>
#include <api/video/video_frame.h>
#include <api/video_codecs/scalability_mode.h>
#include <api/video_codecs/video_codec.h>
#include <common_video/h264/h264_bitstream_parser.h>
#include <modules/video_coding/include/video_codec_interface.h>
#include <modules/video_coding/include/video_error_codes.h>
#include <rtc_base/logging.h>

#include <libyuv/convert.h>
#include <libyuv/convert_from.h>

#include <windows.h>
#include <unknwn.h>
#include <codecapi.h>
// d3d11.h must come first: it pulls in d3d10_1.h, which refuses to follow d3d10.h.
#include <d3d11.h>
#include <d3d10.h>
#include <mfapi.h>
#include <mferror.h>
#include <mfidl.h>
#include <mftransform.h>
#include <strmif.h>

#include <winrt/base.h>

#include <algorithm>
#include <atomic>
#include <cstring>
#include <deque>
#include <functional>
#include <iterator>
#include <mutex>
#include <optional>
#include <set>
#include <thread>
#include <utility>
#include <vector>

namespace gelabber
{
	namespace
	{
		// mfplat.dll is delay-loaded; Windows N without the Media Feature
		// Pack lacks it, and the first MF call would then fail the process.
		bool HasMfPlat()
		{
			static const bool present =
			  LoadLibraryExW(L"mfplat.dll", nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32) != nullptr;
			return present;
		}

		// COM on the calling thread (libwebrtc's encoder and decoder queues
		// have none) and Media Foundation once per process; neither is undone.
		bool EnsureMf()
		{
			if (!HasMfPlat())
				return false;
			const HRESULT com = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
			if (FAILED(com) && com != RPC_E_CHANGED_MODE)
				return false;
			static const bool started = SUCCEEDED(MFStartup(MF_VERSION, MFSTARTUP_LITE));
			return started;
		}

		std::string Utf8(const wchar_t* text)
		{
			if (!text || !*text)
				return {};
			const int size = WideCharToMultiByte(CP_UTF8, 0, text, -1, nullptr, 0, nullptr, nullptr);
			if (size <= 1)
				return {};
			std::string out(static_cast<size_t>(size - 1), '\0');
			WideCharToMultiByte(CP_UTF8, 0, text, -1, out.data(), size, nullptr, nullptr);
			return out;
		}

		std::vector<winrt::com_ptr<IMFActivate>> Enumerate(GUID category, UINT32 flags, GUID input, GUID output)
		{
			const MFT_REGISTER_TYPE_INFO in{ MFMediaType_Video, input };
			const MFT_REGISTER_TYPE_INFO out{ MFMediaType_Video, output };
			IMFActivate** activates = nullptr;
			UINT32 count            = 0;
			std::vector<winrt::com_ptr<IMFActivate>> list;
			if (FAILED(MFTEnumEx(category, flags, &in, &out, &activates, &count)))
				return list;
			for (UINT32 i = 0; i < count; ++i)
			{
				list.emplace_back();
				list.back().attach(activates[i]);
			}
			CoTaskMemFree(activates);
			return list;
		}

		std::string Name(IMFActivate* activate, SystemCodec codec)
		{
			const std::string unnamed = std::string(CodecName(codec)) + " MFT";
			wchar_t* name = nullptr;
			UINT32 length = 0;
			if (FAILED(activate->GetAllocatedString(MFT_FRIENDLY_NAME_Attribute, &name, &length)))
				return unnamed;
			auto out = Utf8(name);
			CoTaskMemFree(name);
			return out.empty() ? unnamed : out;
		}

		// MFVideoFormat_AV1 (FCC('AV01')), spelled out: older Windows SDKs
		// lack it.
		constexpr GUID kAv1Format = { 0x31305641, 0x0000, 0x0010, { 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71 } };

		const GUID& FormatOf(SystemCodec codec)
		{
			return codec == SystemCodec::kH264 ? MFVideoFormat_H264 : kAv1Format;
		}

		std::vector<winrt::com_ptr<IMFActivate>> HardwareEncoders(SystemCodec codec)
		{
			return Enumerate(
			  MFT_CATEGORY_VIDEO_ENCODER,
			  MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER,
			  MFVideoFormat_NV12,
			  FormatOf(codec));
		}

		// AV1 on the CPU is libaom's, not an MFT's.
		std::vector<winrt::com_ptr<IMFActivate>> SoftwareEncoders(SystemCodec codec)
		{
			if (codec != SystemCodec::kH264)
				return {};
			return Enumerate(
			  MFT_CATEGORY_VIDEO_ENCODER,
			  MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER,
			  MFVideoFormat_NV12,
			  FormatOf(codec));
		}

		std::vector<winrt::com_ptr<IMFActivate>> Decoders(SystemCodec codec)
		{
			return Enumerate(
			  MFT_CATEGORY_VIDEO_DECODER,
			  MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER,
			  FormatOf(codec),
			  MFVideoFormat_NV12);
		}

		// Annex B NAL units of `type` (5 IDR, 7 SPS) in `data`.
		bool HasNal(const uint8_t* data, size_t size, uint8_t type)
		{
			for (size_t i = 0; i + 3 < size; ++i)
			{
				if (data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1)
				{
					if ((data[i + 3] & 0x1f) == type)
						return true;
					i += 2;
				}
			}
			return false;
		}

		void SetValue(ICodecAPI* api, const GUID& property, UINT32 value)
		{
			if (!api)
				return;
			VARIANT variant;
			VariantInit(&variant);
			variant.vt    = VT_UI4;
			variant.ulVal = value;
			api->SetValue(&property, &variant);
		}

		void SetFlag(ICodecAPI* api, const GUID& property, bool value)
		{
			if (!api)
				return;
			VARIANT variant;
			VariantInit(&variant);
			variant.vt      = VT_BOOL;
			variant.boolVal = value ? VARIANT_TRUE : VARIANT_FALSE;
			api->SetValue(&property, &variant);
		}

		winrt::com_ptr<IMFSample> NewSample(DWORD size)
		{
			winrt::com_ptr<IMFSample> sample;
			winrt::com_ptr<IMFMediaBuffer> buffer;
			if (FAILED(MFCreateSample(sample.put())) || FAILED(MFCreateMemoryBuffer(size, buffer.put())) ||
			    FAILED(sample->AddBuffer(buffer.get())))
				return nullptr;
			return sample;
		}

		// An AV1 temporal delimiter OBU, with its size field.
		constexpr uint8_t kTemporalDelimiter[] = { av1::kTemporalDelimiter << 3 | 0x02, 0x00 };

		// Inputs without a decoded frame after which a decoder counts as
		// stuck (two seconds at 30 fps).
		constexpr int kStuckAfter = 60;

		enum class Pulled
		{
			Frame,
			NeedInput,
			Changed,
			Failed,
		};

		// One encoder MFT for one picture size. A hardware MFT is
		// asynchronous: it asks for input and announces output through
		// events, which come on a Media Foundation work queue.
		class EncoderSession : public std::enable_shared_from_this<EncoderSession>
		{
		public:
			using Sink = std::function<void(const uint8_t*, size_t, LONGLONG, bool)>;

			struct Events : winrt::implements<Events, IMFAsyncCallback>
			{
				explicit Events(std::weak_ptr<EncoderSession> session) : session(std::move(session))
				{
				}

				HRESULT __stdcall GetParameters(DWORD*, DWORD*) noexcept override
				{
					return E_NOTIMPL;
				}

				HRESULT __stdcall Invoke(IMFAsyncResult* result) noexcept override
				{
					if (auto owner = session.lock())
						owner->OnEvents(result);
					return S_OK;
				}

				std::weak_ptr<EncoderSession> session;
			};

			EncoderSession(SystemCodec codec, Sink sink) : codec(codec), sink(std::move(sink))
			{
			}

			// The first of `candidates` that takes the format.
			bool Open(
			  const std::vector<winrt::com_ptr<IMFActivate>>& candidates,
			  const std::set<std::string>& excluded,
			  int width,
			  int height,
			  int fps,
			  uint32_t bps)
			{
				for (const auto& candidate : candidates)
				{
					const auto name = Name(candidate.get(), codec);
					if (excluded.count(name))
						continue;
					if (Start(candidate, width, height, fps, bps))
					{
						this->name = name;
						return true;
					}
					Shutdown();
				}
				return false;
			}

			bool Push(const winrt::com_ptr<IMFSample>& sample, bool key)
			{
				std::lock_guard io(ioMutex);
				if (!transform || failed)
					return false;
				if (key)
					SetValue(codecApi.get(), CODECAPI_AVEncVideoForceKeyFrame, 1);
				if (!async)
				{
					HRESULT result = transform->ProcessInput(0, sample.get(), 0);
					if (result == MF_E_NOTACCEPTING)
					{
						DrainLocked();
						result = transform->ProcessInput(0, sample.get(), 0);
					}
					if (FAILED(result))
						failed = true;
					else
						DrainLocked();
					return !failed;
				}
				if (needInput > 0)
				{
					--needInput;
					Feed(sample.get());
				}
				else
				{
					// The encoder is behind: keep the newest frames.
					inputs.push_back(sample);
					while (inputs.size() > 2)
						inputs.pop_front();
				}
				return !failed;
			}

			void SetBitrate(uint32_t bps)
			{
				std::lock_guard io(ioMutex);
				SetValue(codecApi.get(), CODECAPI_AVEncCommonMeanBitRate, bps);
			}

			// For good: nothing reaches the sink afterwards.
			void Close()
			{
				{
					std::lock_guard lock(sinkMutex);
					sink = nullptr;
				}
				Shutdown();
			}

			bool Failed() const
			{
				return failed;
			}

			bool Hardware() const
			{
				return async;
			}

			std::string name;

		private:
			void Shutdown()
			{
				std::lock_guard io(ioMutex);
				stopping = true;
				if (transform)
				{
					transform->ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
					transform->ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
					if (auto shutdown = transform.try_as<IMFShutdown>())
						shutdown->Shutdown();
				}
				if (activate)
					activate->ShutdownObject();
				inputs.clear();
				events    = nullptr;
				callback  = nullptr;
				codecApi  = nullptr;
				transform = nullptr;
				activate  = nullptr;
			}

			bool Start(const winrt::com_ptr<IMFActivate>& candidate, int width, int height, int fps, uint32_t bps)
			{
				std::lock_guard io(ioMutex);
				stopping = false;
				failed   = false;
				async    = false;
				activate = candidate;
				if (FAILED(activate->ActivateObject(IID_PPV_ARGS(transform.put()))) || !transform)
					return false;
				winrt::com_ptr<IMFAttributes> attributes;
				if (SUCCEEDED(transform->GetAttributes(attributes.put())) && attributes)
				{
					async = MFGetAttributeUINT32(attributes.get(), MF_TRANSFORM_ASYNC, FALSE) != FALSE;
					if (async && FAILED(attributes->SetUINT32(MF_TRANSFORM_ASYNC_UNLOCK, TRUE)))
						return false;
					attributes->SetUINT32(MF_LOW_LATENCY, TRUE);
				}
				codecApi = transform.try_as<ICodecAPI>();
				SetValue(codecApi.get(), CODECAPI_AVEncCommonRateControlMode, eAVEncCommonRateControlMode_CBR);
				SetValue(codecApi.get(), CODECAPI_AVEncCommonMeanBitRate, bps);
				SetFlag(codecApi.get(), CODECAPI_AVLowLatencyMode, true);
				SetValue(codecApi.get(), CODECAPI_AVEncMPVDefaultBPictureCount, 0);
				// Key frames come when libwebrtc asks for them.
				SetValue(codecApi.get(), CODECAPI_AVEncMPVGOPSize, static_cast<UINT32>(fps) * 60);

				// Encoders take their output type first. H264: Constrained
				// Baseline is what the router and every receiver have. AV1: the
				// profile follows from NV12 (Main, 8 bits, 4:2:0).
				std::vector<std::optional<UINT32>> profiles{ std::nullopt };
				if (codec == SystemCodec::kH264)
					profiles = { static_cast<UINT32>(eAVEncH264VProfile_ConstrainedBase),
						         static_cast<UINT32>(eAVEncH264VProfile_Base) };
				bool output = false;
				for (const auto profile : profiles)
				{
					winrt::com_ptr<IMFMediaType> type;
					if (FAILED(MFCreateMediaType(type.put())))
						return false;
					type->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video);
					type->SetGUID(MF_MT_SUBTYPE, FormatOf(codec));
					type->SetUINT32(MF_MT_AVG_BITRATE, bps);
					type->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive);
					if (profile)
						type->SetUINT32(MF_MT_MPEG2_PROFILE, *profile);
					MFSetAttributeSize(type.get(), MF_MT_FRAME_SIZE, width, height);
					MFSetAttributeRatio(type.get(), MF_MT_FRAME_RATE, fps, 1);
					MFSetAttributeRatio(type.get(), MF_MT_PIXEL_ASPECT_RATIO, 1, 1);
					if (SUCCEEDED(transform->SetOutputType(0, type.get(), 0)))
					{
						output = true;
						break;
					}
				}
				if (!output)
					return false;

				// NV12 in the form the encoder offers it.
				winrt::com_ptr<IMFMediaType> input;
				for (DWORD index = 0;; ++index)
				{
					winrt::com_ptr<IMFMediaType> type;
					if (FAILED(transform->GetInputAvailableType(0, index, type.put())))
						break;
					GUID subtype{};
					if (SUCCEEDED(type->GetGUID(MF_MT_SUBTYPE, &subtype)) && subtype == MFVideoFormat_NV12)
					{
						input = type;
						break;
					}
				}
				if (!input && FAILED(MFCreateMediaType(input.put())))
					return false;
				input->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video);
				input->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_NV12);
				input->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive);
				MFSetAttributeSize(input.get(), MF_MT_FRAME_SIZE, width, height);
				MFSetAttributeRatio(input.get(), MF_MT_FRAME_RATE, fps, 1);
				MFSetAttributeRatio(input.get(), MF_MT_PIXEL_ASPECT_RATIO, 1, 1);
				if (FAILED(transform->SetInputType(0, input.get(), 0)))
					return false;

				MFT_OUTPUT_STREAM_INFO info{};
				if (FAILED(transform->GetOutputStreamInfo(0, &info)))
					return false;
				providesSamples =
				  (info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES)) != 0;
				outputSize = std::max<DWORD>(info.cbSize, static_cast<DWORD>(width) * height * 3 / 2);
				ReadSequenceHeader();

				if (FAILED(transform->ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)) ||
				    FAILED(transform->ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)))
					return false;
				needInput = 0;
				if (async)
				{
					events = transform.try_as<IMFMediaEventGenerator>();
					if (!events)
						return false;
					callback = winrt::make_self<Events>(weak_from_this());
					if (FAILED(events->BeginGetEvent(callback.get(), nullptr)))
						return false;
				}
				return true;
			}

			// H264: SPS/PPS in Annex B. AV1: the sequence header OBU, from the
			// blob itself or from the configOBUs of an AV1CodecConfigurationRecord
			// (av1C, first byte 0x81).
			void ReadSequenceHeader()
			{
				header.clear();
				winrt::com_ptr<IMFMediaType> current;
				UINT32 size = 0;
				if (SUCCEEDED(transform->GetOutputCurrentType(0, current.put())) &&
				    SUCCEEDED(current->GetBlobSize(MF_MT_MPEG_SEQUENCE_HEADER, &size)) && size > 0)
				{
					header.resize(size);
					if (FAILED(current->GetBlob(MF_MT_MPEG_SEQUENCE_HEADER, header.data(), size, nullptr)))
						header.clear();
				}
				if (codec == SystemCodec::kAv1 && !header.empty())
				{
					const size_t skip = header.size() > 4 && header[0] == 0x81 ? 4 : 0;
					header            = av1::SequenceHeader(header.data() + skip, header.size() - skip);
				}
			}

			void OnEvents(IMFAsyncResult* result)
			{
				winrt::com_ptr<IMFMediaEventGenerator> generator;
				{
					std::lock_guard io(ioMutex);
					generator = events;
				}
				winrt::com_ptr<IMFMediaEvent> event;
				// Fails once the MFT is shut down: no new request then.
				if (!generator || FAILED(generator->EndGetEvent(result, event.put())))
					return;
				MediaEventType type = MEUnknown;
				event->GetType(&type);
				winrt::com_ptr<Events> again;
				{
					std::lock_guard io(ioMutex);
					if (!transform || stopping)
						return;
					switch (type)
					{
					case METransformNeedInput:
						if (!inputs.empty())
						{
							auto sample = inputs.front();
							inputs.pop_front();
							Feed(sample.get());
						}
						else
							++needInput;
						break;
					case METransformHaveOutput:
						while (PullLocked() == Pulled::Changed)
						{
						}
						break;
					case MEError:
						failed = true;
						break;
					default:
						break;
					}
					if (failed)
						return;
					again = callback;
				}
				if (again)
					generator->BeginGetEvent(again.get(), nullptr);
			}

			void Feed(IMFSample* sample)
			{
				if (FAILED(transform->ProcessInput(0, sample, 0)))
					failed = true;
			}

			void DrainLocked()
			{
				for (int round = 0; round < 32; ++round)
				{
					const auto pulled = PullLocked();
					if (pulled == Pulled::NeedInput || pulled == Pulled::Failed)
						return;
				}
			}

			Pulled PullLocked()
			{
				MFT_OUTPUT_DATA_BUFFER out{};
				winrt::com_ptr<IMFSample> own;
				if (!providesSamples)
				{
					own = NewSample(outputSize);
					if (!own)
						return Pulled::Failed;
					out.pSample = own.get();
				}
				DWORD status        = 0;
				const HRESULT result = transform->ProcessOutput(0, 1, &out, &status);
				if (out.pEvents)
					out.pEvents->Release();
				winrt::com_ptr<IMFSample> sample;
				if (!own && out.pSample)
					sample.attach(out.pSample);
				else
					sample = own;
				if (result == MF_E_TRANSFORM_NEED_MORE_INPUT)
					return Pulled::NeedInput;
				if (result == MF_E_TRANSFORM_STREAM_CHANGE)
				{
					winrt::com_ptr<IMFMediaType> type;
					if (FAILED(transform->GetOutputAvailableType(0, 0, type.put())) ||
					    FAILED(transform->SetOutputType(0, type.get(), 0)))
					{
						failed = true;
						return Pulled::Failed;
					}
					ReadSequenceHeader();
					return Pulled::Changed;
				}
				if (FAILED(result) || !sample)
				{
					failed = true;
					return Pulled::Failed;
				}
				Deliver(sample.get());
				return Pulled::Frame;
			}

			void Deliver(IMFSample* sample)
			{
				LONGLONG time = 0;
				sample->GetSampleTime(&time);
				const bool clean = MFGetAttributeUINT32(sample, MFSampleExtension_CleanPoint, FALSE) != FALSE;
				winrt::com_ptr<IMFMediaBuffer> buffer;
				BYTE* data   = nullptr;
				DWORD length = 0;
				if (FAILED(sample->ConvertToContiguousBuffer(buffer.put())) ||
				    FAILED(buffer->Lock(&data, nullptr, &length)))
					return;
				bool key;
				std::vector<uint8_t> bytes;
				if (codec == SystemCodec::kH264)
				{
					key = clean || HasNal(data, length, 5);
					// Some encoders keep SPS/PPS in the media type only.
					if (key && !header.empty() && !HasNal(data, length, 7))
						bytes.insert(bytes.end(), header.begin(), header.end());
					bytes.insert(bytes.end(), data, data + length);
				}
				else
				{
					key = clean;
					// The last sequence header goes in front of a key frame
					// that comes without one.
					if (auto inBand = av1::SequenceHeader(data, length); !inBand.empty())
						header = std::move(inBand);
					bytes = key ? av1::WithSequenceHeader(data, length, header)
					            : std::vector<uint8_t>(data, data + length);
				}
				buffer->Unlock();
				std::lock_guard lock(sinkMutex);
				if (sink && !bytes.empty())
					sink(bytes.data(), bytes.size(), time, key);
			}

			const SystemCodec codec;
			std::mutex sinkMutex;
			Sink sink;
			std::mutex ioMutex;
			winrt::com_ptr<IMFActivate> activate;
			winrt::com_ptr<IMFTransform> transform;
			winrt::com_ptr<ICodecAPI> codecApi;
			winrt::com_ptr<IMFMediaEventGenerator> events;
			winrt::com_ptr<Events> callback;
			std::deque<winrt::com_ptr<IMFSample>> inputs;
			std::vector<uint8_t> header;
			int needInput{ 0 };
			bool async{ false };
			bool providesSamples{ false };
			DWORD outputSize{ 0 };
			bool stopping{ false };
			std::atomic<bool> failed{ false };
		};

		// One picture size on one MFT, like libwebrtc's single-stream
		// encoders. A failing MFT is replaced by the next candidate. AV1 runs
		// behind libaom (HardwareFirstEncoder), which takes over once no MFT
		// is left.
		class MfEncoder : public webrtc::VideoEncoder
		{
		public:
			explicit MfEncoder(SystemCodec codec) : codec(codec)
			{
			}

			~MfEncoder() override
			{
				Release();
			}

			int InitEncode(const webrtc::VideoCodec* settings, const Settings&) override
			{
				Release();
				if (!settings || settings->codecType != Type() || settings->width < 2 || settings->height < 2 ||
				    settings->numberOfSimulcastStreams > 1 || !EnsureMf())
					return WEBRTC_VIDEO_CODEC_ERR_PARAMETER;
				// AV1 in one spatial and one temporal layer; libaom does the others.
				if (codec == SystemCodec::kAv1 &&
				    settings->GetScalabilityMode().value_or(webrtc::ScalabilityMode::kL1T1) != webrtc::ScalabilityMode::kL1T1)
					return WEBRTC_VIDEO_CODEC_FALLBACK_SOFTWARE;
				width  = settings->width & ~1;
				height = settings->height & ~1;
				fps    = std::max<int>(1, settings->maxFramerate);
				bitrate.store(std::max<uint32_t>(settings->startBitrate, 100) * 1000);
				excluded.clear();
				return Open() ? WEBRTC_VIDEO_CODEC_OK : NoEncoder();
			}

			int32_t RegisterEncodeCompleteCallback(webrtc::EncodedImageCallback* callback) override
			{
				std::lock_guard lock(mutex);
				this->callback = callback;
				return WEBRTC_VIDEO_CODEC_OK;
			}

			int32_t Release() override
			{
				if (session)
					session->Close();
				session.reset();
				std::lock_guard lock(mutex);
				pending.clear();
				firstUs.reset();
				return WEBRTC_VIDEO_CODEC_OK;
			}

			int32_t Encode(
			  const webrtc::VideoFrame& frame, const std::vector<webrtc::VideoFrameType>* types) override
			{
				if (!session)
					return WEBRTC_VIDEO_CODEC_UNINITIALIZED;
				if (session->Failed())
				{
					RTC_LOG(LS_WARNING) << "Media Foundation " << session->name << " failed";
					excluded.insert(session->name);
					session->Close();
					session.reset();
					if (!Open())
						return NoEncoder();
					forceKey = true;
				}
				const uint32_t target = bitrate.load();
				if (target == 0)
					return WEBRTC_VIDEO_CODEC_OK;
				if (target != appliedBitrate)
				{
					session->SetBitrate(target);
					appliedBitrate = target;
				}
				bool key = std::exchange(forceKey, false);
				if (types)
					key = key || std::any_of(types->begin(), types->end(), [](auto type) {
						      return type == webrtc::VideoFrameType::kVideoFrameKey;
					      });

				auto i420 = frame.video_frame_buffer()->ToI420();
				if (!i420)
					return WEBRTC_VIDEO_CODEC_ERROR;
				if (i420->width() != width || i420->height() != height)
				{
					auto scaled = webrtc::I420Buffer::Create(width, height);
					scaled->ScaleFrom(*i420);
					i420 = scaled;
				}
				const DWORD ySize = static_cast<DWORD>(width) * height;
				auto sample       = NewSample(ySize + ySize / 2);
				winrt::com_ptr<IMFMediaBuffer> buffer;
				BYTE* data = nullptr;
				if (!sample || FAILED(sample->GetBufferByIndex(0, buffer.put())) ||
				    FAILED(buffer->Lock(&data, nullptr, nullptr)))
					return WEBRTC_VIDEO_CODEC_ERROR;
				libyuv::I420ToNV12(
				  i420->DataY(),
				  i420->StrideY(),
				  i420->DataU(),
				  i420->StrideU(),
				  i420->DataV(),
				  i420->StrideV(),
				  data,
				  width,
				  data + ySize,
				  width,
				  width,
				  height);
				buffer->Unlock();
				buffer->SetCurrentLength(ySize + ySize / 2);

				LONGLONG time;
				{
					std::lock_guard lock(mutex);
					const int64_t nowUs = frame.timestamp_us();
					if (!firstUs)
						firstUs = nowUs;
					time = std::max<int64_t>(0, nowUs - *firstUs) * 10;
					if (!pending.empty() && time <= pending.back().time)
						time = pending.back().time + 1;
					pending.push_back({ time,
					                    frame.rtp_timestamp(),
					                    frame.ntp_time_ms(),
					                    frame.render_time_ms(),
					                    frame.rotation(),
					                    frame.color_space() });
					// Bounded: the encoder may drop frames.
					while (pending.size() > 64)
						pending.pop_front();
				}
				sample->SetSampleTime(time);
				sample->SetSampleDuration(10'000'000 / fps);
				// Without the lock: a synchronous MFT delivers from inside.
				if (!session->Push(sample, key))
				{
					forceKey = true;
					return WEBRTC_VIDEO_CODEC_ERROR;
				}
				return WEBRTC_VIDEO_CODEC_OK;
			}

			void SetRates(const RateControlParameters& parameters) override
			{
				bitrate.store(parameters.bitrate.get_sum_bps());
			}

			EncoderInfo GetEncoderInfo() const override
			{
				EncoderInfo info;
				info.supports_native_handle         = false;
				info.implementation_name =
				  "Media Foundation " + (session ? session->name : std::string(CodecName(codec)));
				info.is_hardware_accelerated        = session && session->Hardware();
				info.supports_simulcast             = false;
				info.requested_resolution_alignment = 2;
				// libwebrtc reads no QP from AV1 (see Deliver).
				info.scaling_settings =
				  codec == SystemCodec::kH264 ? ScalingSettings(24, 37) : ScalingSettings(ScalingSettings::kOff);
				info.preferred_pixel_formats        = { webrtc::VideoFrameBuffer::Type::kI420,
					                                    webrtc::VideoFrameBuffer::Type::kNV12 };
				return info;
			}

		private:
			struct Pending
			{
				LONGLONG time;
				uint32_t rtpTimestamp;
				int64_t ntpMs;
				int64_t renderMs;
				webrtc::VideoRotation rotation;
				std::optional<webrtc::ColorSpace> colorSpace;
			};

			webrtc::VideoCodecType Type() const
			{
				return codec == SystemCodec::kH264 ? webrtc::kVideoCodecH264 : webrtc::kVideoCodecAV1;
			}

			// Without an MFT: AV1 hands over to libaom, H264 has nothing else.
			int NoEncoder() const
			{
				return codec == SystemCodec::kH264 ? WEBRTC_VIDEO_CODEC_ERROR : WEBRTC_VIDEO_CODEC_FALLBACK_SOFTWARE;
			}

			bool Open()
			{
				auto next = std::make_shared<EncoderSession>(
				  codec,
				  [this](const uint8_t* data, size_t size, LONGLONG time, bool key) { Deliver(data, size, time, key); });
				if (!next->Open(HardwareEncoders(codec), excluded, width, height, fps, bitrate.load()) &&
				    !next->Open(SoftwareEncoders(codec), excluded, width, height, fps, bitrate.load()))
				{
					RTC_LOG(LS_WARNING) << "no Media Foundation " << CodecName(codec) << " encoder takes " << width
					                    << "x" << height;
					return false;
				}
				appliedBitrate = bitrate.load();
				session        = std::move(next);
				RTC_LOG(LS_INFO) << "Media Foundation " << CodecName(codec) << " " << session->name << " " << width
				                 << "x" << height << "@" << fps;
				return true;
			}

			void Deliver(const uint8_t* data, size_t size, LONGLONG time, bool key)
			{
				std::lock_guard lock(mutex);
				// Frames the encoder dropped stay in front of the match.
				while (!pending.empty() && pending.front().time < time)
					pending.pop_front();
				if (pending.empty() || pending.front().time != time || !callback)
					return;
				const Pending meta = pending.front();
				pending.pop_front();

				webrtc::EncodedImage image;
				image.SetEncodedData(webrtc::EncodedImageBuffer::Create(data, size));
				image._encodedWidth  = width;
				image._encodedHeight = height;
				image.SetRtpTimestamp(meta.rtpTimestamp);
				image.ntp_time_ms_     = meta.ntpMs;
				image.capture_time_ms_ = meta.renderMs;
				image.rotation_        = meta.rotation;
				image.SetColorSpace(meta.colorSpace);
				image._frameType =
				  key ? webrtc::VideoFrameType::kVideoFrameKey : webrtc::VideoFrameType::kVideoFrameDelta;

				webrtc::CodecSpecificInfo info;
				info.codecType = Type();
				if (codec == SystemCodec::kH264)
				{
					parser.ParseBitstream(image);
					image.qp_ = parser.GetLastSliceQp().value_or(-1);
					info.codecSpecific.H264.packetization_mode = webrtc::H264PacketizationMode::NonInterleaved;
					info.codecSpecific.H264.temporal_idx       = webrtc::kNoTemporalIdx;
					info.codecSpecific.H264.idr_frame          = key;
					info.codecSpecific.H264.base_layer_sync    = false;
				}
				callback->OnEncodedImage(image, &info);
			}

			const SystemCodec codec;
			std::shared_ptr<EncoderSession> session;
			std::set<std::string> excluded;
			int width{ 0 };
			int height{ 0 };
			int fps{ 30 };
			std::atomic<uint32_t> bitrate{ 0 };
			uint32_t appliedBitrate{ 0 };
			bool forceKey{ false };
			std::mutex mutex;
			webrtc::EncodedImageCallback* callback{ nullptr };
			std::deque<Pending> pending;
			std::optional<int64_t> firstUs;
			webrtc::H264BitstreamParser parser;
		};

		// Simulcast without libwebrtc's SimulcastEncoderAdapter, which the
		// Windows package does not carry: one encoder per layer, each fed
		// the frame scaled to its layer.
		class LayeredEncoder : public webrtc::VideoEncoder
		{
		public:
			~LayeredEncoder() override
			{
				Release();
			}

			int InitEncode(const webrtc::VideoCodec* codec, const Settings& settings) override
			{
				Release();
				if (!codec)
					return WEBRTC_VIDEO_CODEC_ERR_PARAMETER;
				const int count = std::max<int>(1, codec->numberOfSimulcastStreams);
				for (int index = 0; index < count; ++index)
				{
					webrtc::VideoCodec single = *codec;
					if (codec->numberOfSimulcastStreams > 1)
					{
						const auto& stream               = codec->simulcastStream[index];
						single.width                     = static_cast<uint16_t>(stream.width);
						single.height                    = static_cast<uint16_t>(stream.height);
						single.maxBitrate                = stream.maxBitrate;
						single.minBitrate                = stream.minBitrate;
						single.startBitrate              = std::min(codec->startBitrate, stream.targetBitrate);
						single.maxFramerate              = static_cast<uint32_t>(std::max(1.0f, stream.maxFramerate));
						single.numberOfSimulcastStreams  = 1;
						single.simulcastStream[0]        = stream;
					}
					auto layer      = std::make_unique<Layer>();
					layer->index    = index;
					layer->owner    = this;
					layer->width    = single.width;
					layer->height   = single.height;
					layer->encoder  = std::make_unique<MfEncoder>(SystemCodec::kH264);
					layer->encoder->RegisterEncodeCompleteCallback(layer.get());
					const int result = layer->encoder->InitEncode(&single, settings);
					if (result != WEBRTC_VIDEO_CODEC_OK)
					{
						Release();
						return result;
					}
					layer->active = codec->numberOfSimulcastStreams <= 1 || codec->simulcastStream[index].active;
					layers.push_back(std::move(layer));
				}
				return WEBRTC_VIDEO_CODEC_OK;
			}

			int32_t RegisterEncodeCompleteCallback(webrtc::EncodedImageCallback* callback) override
			{
				std::lock_guard lock(mutex);
				this->callback = callback;
				return WEBRTC_VIDEO_CODEC_OK;
			}

			int32_t Release() override
			{
				for (auto& layer : layers)
					layer->encoder->Release();
				layers.clear();
				return WEBRTC_VIDEO_CODEC_OK;
			}

			int32_t Encode(
			  const webrtc::VideoFrame& frame, const std::vector<webrtc::VideoFrameType>* types) override
			{
				if (layers.empty())
					return WEBRTC_VIDEO_CODEC_UNINITIALIZED;
				int32_t result = WEBRTC_VIDEO_CODEC_OK;
				for (auto& layer : layers)
				{
					if (!layer->active)
						continue;
					const bool key = types && static_cast<size_t>(layer->index) < types->size() &&
					                 (*types)[layer->index] == webrtc::VideoFrameType::kVideoFrameKey;
					const std::vector<webrtc::VideoFrameType> layerTypes{
						key ? webrtc::VideoFrameType::kVideoFrameKey : webrtc::VideoFrameType::kVideoFrameDelta
					};
					int32_t encoded;
					if (frame.width() == layer->width && frame.height() == layer->height)
						encoded = layer->encoder->Encode(frame, &layerTypes);
					else
					{
						auto source = frame.video_frame_buffer()->ToI420();
						if (!source)
							return WEBRTC_VIDEO_CODEC_ERROR;
						auto scaled = webrtc::I420Buffer::Create(layer->width, layer->height);
						scaled->ScaleFrom(*source);
						const auto layerFrame = webrtc::VideoFrame::Builder()
						                          .set_video_frame_buffer(scaled)
						                          .set_rtp_timestamp(frame.rtp_timestamp())
						                          .set_timestamp_us(frame.timestamp_us())
						                          .set_ntp_time_ms(frame.ntp_time_ms())
						                          .set_rotation(frame.rotation())
						                          .set_color_space(frame.color_space())
						                          .build();
						encoded = layer->encoder->Encode(layerFrame, &layerTypes);
					}
					if (encoded != WEBRTC_VIDEO_CODEC_OK)
						result = encoded;
				}
				return result;
			}

			void SetRates(const RateControlParameters& parameters) override
			{
				for (auto& layer : layers)
				{
					const uint32_t bps =
					  layers.size() == 1 ? parameters.bitrate.get_sum_bps()
					                     : parameters.bitrate.GetSpatialLayerSum(layer->index);
					webrtc::VideoBitrateAllocation allocation;
					allocation.SetBitrate(0, 0, bps);
					RateControlParameters single = parameters;
					single.bitrate               = allocation;
					layer->active                = bps > 0;
					layer->encoder->SetRates(single);
				}
			}

			EncoderInfo GetEncoderInfo() const override
			{
				EncoderInfo info = layers.empty() ? MfEncoder(SystemCodec::kH264).GetEncoderInfo()
				                                  : layers.front()->encoder->GetEncoderInfo();
				info.supports_simulcast                     = true;
				info.apply_alignment_to_all_simulcast_layers = true;
				return info;
			}

		private:
			struct Layer : webrtc::EncodedImageCallback
			{
				Result OnEncodedImage(
				  const webrtc::EncodedImage& image, const webrtc::CodecSpecificInfo* info) override
				{
					webrtc::EncodedImage indexed = image;
					indexed.SetSimulcastIndex(owner->layers.size() > 1 ? std::optional<int>(index) : std::nullopt);
					std::lock_guard lock(owner->mutex);
					if (!owner->callback)
						return Result(Result::ERROR_SEND_FAILED);
					return owner->callback->OnEncodedImage(indexed, info);
				}

				int index{ 0 };
				LayeredEncoder* owner{ nullptr };
				int width{ 0 };
				int height{ 0 };
				bool active{ true };
				std::unique_ptr<MfEncoder> encoder;
			};

			std::vector<std::unique_ptr<Layer>> layers;
			std::mutex mutex;
			webrtc::EncodedImageCallback* callback{ nullptr };
		};

		// Microsoft's H264 decoder MFT, on the GPU through DXVA when it gets
		// a Direct3D 11 device. Synchronous: frames come out of Decode().
		// AV1: Microsoft's AV1 decoder (AV1 Video Extension) only where it
		// decodes through DXVA, behind dav1d (HardwareFirstDecoder), which
		// takes over when it fails or puts out no frames.
		class MfDecoder : public webrtc::VideoDecoder
		{
		public:
			explicit MfDecoder(SystemCodec codec) : codec(codec)
			{
			}

			~MfDecoder() override
			{
				Release();
			}

			bool Configure(const Settings& settings) override
			{
				Release();
				if (!EnsureMf())
					return false;
				// Where the stream says nothing yet; the first key frame sets
				// the real size (MF_E_TRANSFORM_STREAM_CHANGE).
				const auto size = settings.max_render_resolution();
				const UINT32 width  = size.Valid() ? static_cast<UINT32>(size.Width()) : 1280;
				const UINT32 height = size.Valid() ? static_cast<UINT32>(size.Height()) : 720;
				for (const auto& candidate : Decoders(codec))
				{
					// dav1d on the CPU beats an MFT on the CPU.
					if (Start(candidate, width, height) && (codec == SystemCodec::kH264 || dxva))
					{
						name = Name(candidate.get(), codec);
						RTC_LOG(LS_INFO) << "Media Foundation " << CodecName(codec) << " decoder " << name
						                 << (dxva ? " (DXVA)" : "");
						return true;
					}
					Release();
				}
				return false;
			}

			int32_t Decode(const webrtc::EncodedImage& input, int64_t renderTimeMs) override
			{
				if (!transform)
					return WEBRTC_VIDEO_CODEC_UNINITIALIZED;
				if (!input.data() || input.size() == 0)
					return WEBRTC_VIDEO_CODEC_ERR_PARAMETER;
				// AV1: libwebrtc's temporal units come without the temporal
				// delimiter the bitstream format starts each of them with.
				const bool delimit = codec == SystemCodec::kAv1 &&
				                     ((input.data()[0] >> 3) & 0x0f) != av1::kTemporalDelimiter;
				const size_t size  = input.size() + (delimit ? sizeof(kTemporalDelimiter) : 0);
				auto sample = NewSample(static_cast<DWORD>(size));
				winrt::com_ptr<IMFMediaBuffer> buffer;
				BYTE* data = nullptr;
				if (!sample || FAILED(sample->GetBufferByIndex(0, buffer.put())) ||
				    FAILED(buffer->Lock(&data, nullptr, nullptr)))
					return WEBRTC_VIDEO_CODEC_ERROR;
				if (delimit)
					std::memcpy(data, kTemporalDelimiter, sizeof(kTemporalDelimiter));
				std::memcpy(data + (delimit ? sizeof(kTemporalDelimiter) : 0), input.data(), input.size());
				buffer->Unlock();
				buffer->SetCurrentLength(static_cast<DWORD>(size));
				// Only the order counts: one millisecond per frame.
				const LONGLONG time = static_cast<LONGLONG>(++count) * 10'000;
				sample->SetSampleTime(time);
				const auto* colorSpace = input.ColorSpace();
				pending.push_back({ time,
				                    input.RtpTimestamp(),
				                    input.NtpTimeMs(),
				                    renderTimeMs,
				                    input.rotation(),
				                    colorSpace ? std::optional(*colorSpace) : std::nullopt });
				while (pending.size() > 64)
					pending.pop_front();

				HRESULT result = transform->ProcessInput(0, sample.get(), 0);
				if (result == MF_E_NOTACCEPTING)
				{
					if (!Drain())
						return Failed();
					result = transform->ProcessInput(0, sample.get(), 0);
				}
				if (FAILED(result) || !Drain())
					return Failed();
				// A decoder that stops putting out frames without an error
				// hands over like a failed one.
				if (codec == SystemCodec::kAv1 && ++sinceOutput > kStuckAfter)
				{
					RTC_LOG(LS_WARNING) << "Media Foundation " << name << " puts out no frames";
					return Failed();
				}
				return WEBRTC_VIDEO_CODEC_OK;
			}

			int32_t RegisterDecodeCompleteCallback(webrtc::DecodedImageCallback* callback) override
			{
				this->callback = callback;
				return WEBRTC_VIDEO_CODEC_OK;
			}

			int32_t Release() override
			{
				if (transform)
				{
					transform->ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
					transform->ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
				}
				if (activate)
					activate->ShutdownObject();
				transform = nullptr;
				activate  = nullptr;
				manager   = nullptr;
				device    = nullptr;
				dxva      = false;
				pending.clear();
				sinceOutput = 0;
				return WEBRTC_VIDEO_CODEC_OK;
			}

			DecoderInfo GetDecoderInfo() const override
			{
				DecoderInfo info;
				info.implementation_name     = "Media Foundation " + name + (dxva ? " (DXVA)" : "");
				info.is_hardware_accelerated = dxva;
				return info;
			}

		private:
			struct Pending
			{
				LONGLONG time;
				uint32_t rtpTimestamp;
				int64_t ntpMs;
				int64_t renderMs;
				webrtc::VideoRotation rotation;
				std::optional<webrtc::ColorSpace> colorSpace;
			};

			// AV1 hands over to dav1d; H264 has nothing else, and the error
			// makes libwebrtc ask for a key frame.
			int32_t Failed() const
			{
				return codec == SystemCodec::kH264 ? WEBRTC_VIDEO_CODEC_ERROR : WEBRTC_VIDEO_CODEC_FALLBACK_SOFTWARE;
			}

			bool Start(const winrt::com_ptr<IMFActivate>& candidate, UINT32 width, UINT32 height)
			{
				activate = candidate;
				if (FAILED(activate->ActivateObject(IID_PPV_ARGS(transform.put()))) || !transform)
					return false;
				winrt::com_ptr<IMFAttributes> attributes;
				if (SUCCEEDED(transform->GetAttributes(attributes.put())) && attributes)
				{
					attributes->SetUINT32(MF_LOW_LATENCY, TRUE);
					if (MFGetAttributeUINT32(attributes.get(), MF_SA_D3D11_AWARE, FALSE))
						dxva = UseGpu();
				}
				winrt::com_ptr<IMFMediaType> input;
				if (FAILED(MFCreateMediaType(input.put())))
					return false;
				input->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video);
				input->SetGUID(MF_MT_SUBTYPE, FormatOf(codec));
				input->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive);
				MFSetAttributeSize(input.get(), MF_MT_FRAME_SIZE, width, height);
				if (FAILED(transform->SetInputType(0, input.get(), 0)) || !SelectOutput())
					return false;
				return SUCCEEDED(transform->ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)) &&
				       SUCCEEDED(transform->ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0));
			}

			// A Direct3D 11 device for DXVA; without one the MFT decodes on
			// the CPU.
			bool UseGpu()
			{
				const D3D_FEATURE_LEVEL levels[] = { D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0,
					                                 D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0 };
				if (FAILED(D3D11CreateDevice(
				      nullptr,
				      D3D_DRIVER_TYPE_HARDWARE,
				      nullptr,
				      D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
				      levels,
				      static_cast<UINT>(std::size(levels)),
				      D3D11_SDK_VERSION,
				      device.put(),
				      nullptr,
				      nullptr)))
					return false;
				if (auto multithread = device.try_as<ID3D10Multithread>())
					multithread->SetMultithreadProtected(TRUE);
				UINT token = 0;
				if (FAILED(MFCreateDXGIDeviceManager(&token, manager.put())) ||
				    FAILED(manager->ResetDevice(device.get(), token)) ||
				    FAILED(transform->ProcessMessage(
				      MFT_MESSAGE_SET_D3D_MANAGER, reinterpret_cast<ULONG_PTR>(manager.get()))))
				{
					manager = nullptr;
					device  = nullptr;
					return false;
				}
				return true;
			}

			// NV12 output; its size, and the picture inside it.
			bool SelectOutput()
			{
				for (DWORD index = 0;; ++index)
				{
					winrt::com_ptr<IMFMediaType> type;
					if (FAILED(transform->GetOutputAvailableType(0, index, type.put())))
						return false;
					GUID subtype{};
					if (FAILED(type->GetGUID(MF_MT_SUBTYPE, &subtype)) || subtype != MFVideoFormat_NV12)
						continue;
					if (FAILED(transform->SetOutputType(0, type.get(), 0)))
						return false;
					UINT32 width = 0, height = 0;
					MFGetAttributeSize(type.get(), MF_MT_FRAME_SIZE, &width, &height);
					frameWidth = static_cast<int>(width);
					frameHeight = static_cast<int>(height);
					pictureWidth  = frameWidth;
					pictureHeight = frameHeight;
					MFVideoArea area{};
					if (SUCCEEDED(type->GetBlob(
					      MF_MT_MINIMUM_DISPLAY_APERTURE, reinterpret_cast<UINT8*>(&area), sizeof(area), nullptr)) &&
					    area.Area.cx > 0 && area.Area.cy > 0)
					{
						pictureWidth  = std::min<int>(area.Area.cx, frameWidth);
						pictureHeight = std::min<int>(area.Area.cy, frameHeight);
					}
					stride = static_cast<int>(MFGetAttributeUINT32(type.get(), MF_MT_DEFAULT_STRIDE, width));
					MFT_OUTPUT_STREAM_INFO info{};
					if (FAILED(transform->GetOutputStreamInfo(0, &info)))
						return false;
					providesSamples =
					  (info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES)) != 0;
					outputSize = std::max<DWORD>(info.cbSize, width * height * 3 / 2);
					return frameWidth > 0 && frameHeight > 0;
				}
			}

			// Every frame the MFT has; false when it failed.
			bool Drain()
			{
				for (int round = 0; round < 32; ++round)
				{
					MFT_OUTPUT_DATA_BUFFER out{};
					winrt::com_ptr<IMFSample> own;
					if (!providesSamples)
					{
						own = NewSample(outputSize);
						if (!own)
							return false;
						out.pSample = own.get();
					}
					DWORD status         = 0;
					const HRESULT result = transform->ProcessOutput(0, 1, &out, &status);
					if (out.pEvents)
						out.pEvents->Release();
					winrt::com_ptr<IMFSample> sample;
					if (!own && out.pSample)
						sample.attach(out.pSample);
					else
						sample = own;
					if (result == MF_E_TRANSFORM_NEED_MORE_INPUT)
						return true;
					if (result == MF_E_TRANSFORM_STREAM_CHANGE)
					{
						if (!SelectOutput())
							return false;
						continue;
					}
					if (FAILED(result) || !sample)
						return false;
					Deliver(sample.get());
				}
				return true;
			}

			void Deliver(IMFSample* sample)
			{
				LONGLONG time = 0;
				sample->GetSampleTime(&time);
				while (!pending.empty() && pending.front().time < time)
					pending.pop_front();
				if (pending.empty() || pending.front().time != time || !callback)
					return;
				const Pending meta = pending.front();
				pending.pop_front();
				sinceOutput = 0;

				winrt::com_ptr<IMFMediaBuffer> buffer;
				if (FAILED(sample->GetBufferByIndex(0, buffer.put())))
					return;
				BYTE* data  = nullptr;
				LONG pitch  = stride;
				auto planar = buffer.try_as<IMF2DBuffer>();
				DWORD length = 0;
				if (planar)
				{
					if (FAILED(planar->Lock2D(&data, &pitch)))
						return;
				}
				else if (FAILED(buffer->Lock(&data, nullptr, &length)))
					return;
				auto i420 = webrtc::I420Buffer::Create(pictureWidth, pictureHeight);
				const uint8_t* y  = data;
				const uint8_t* uv = data + static_cast<size_t>(pitch) * frameHeight;
				libyuv::NV12ToI420(
				  y,
				  pitch,
				  uv,
				  pitch,
				  i420->MutableDataY(),
				  i420->StrideY(),
				  i420->MutableDataU(),
				  i420->StrideU(),
				  i420->MutableDataV(),
				  i420->StrideV(),
				  pictureWidth,
				  pictureHeight);
				if (planar)
					planar->Unlock2D();
				else
					buffer->Unlock();
				auto frame = webrtc::VideoFrame::Builder()
				               .set_video_frame_buffer(i420)
				               .set_rtp_timestamp(meta.rtpTimestamp)
				               .set_timestamp_ms(meta.renderMs)
				               .set_ntp_time_ms(meta.ntpMs)
				               .set_rotation(meta.rotation)
				               .set_color_space(meta.colorSpace)
				               .build();
				callback->Decoded(frame, std::nullopt, std::nullopt);
			}

			const SystemCodec codec;
			winrt::com_ptr<IMFActivate> activate;
			winrt::com_ptr<IMFTransform> transform;
			winrt::com_ptr<ID3D11Device> device;
			winrt::com_ptr<IMFDXGIDeviceManager> manager;
			std::string name;
			bool dxva{ false };
			bool providesSamples{ false };
			DWORD outputSize{ 0 };
			int frameWidth{ 0 };
			int frameHeight{ 0 };
			int pictureWidth{ 0 };
			int pictureHeight{ 0 };
			int stride{ 0 };
			uint64_t count{ 0 };
			int sinceOutput{ 0 };
			webrtc::DecodedImageCallback* callback{ nullptr };
			std::deque<Pending> pending;
		};

		// DXVA decoder profiles (dxva.h), here so that an older SDK builds.
		constexpr GUID kH264DecodeProfile = {
			0x1b81be68, 0xa0c7, 0x11d3, { 0xb9, 0x84, 0x00, 0xc0, 0x4f, 0x2e, 0x73, 0xc5 }
		};
		constexpr GUID kAv1DecodeProfile = {
			0xb8be4ccb, 0xcf53, 0x46ba, { 0x8d, 0x59, 0xd6, 0xb8, 0xa6, 0xda, 0x5d, 0x2a }
		};

		// Whether the GPU decodes `codec` (8-bit 4:2:0), which is what the
		// decoder MFT uses through DXVA.
		bool GpuDecodes(SystemCodec codec)
		{
			winrt::com_ptr<ID3D11Device> device;
			if (FAILED(D3D11CreateDevice(
			      nullptr,
			      D3D_DRIVER_TYPE_HARDWARE,
			      nullptr,
			      D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
			      nullptr,
			      0,
			      D3D11_SDK_VERSION,
			      device.put(),
			      nullptr,
			      nullptr)))
				return false;
			const auto video = device.try_as<ID3D11VideoDevice>();
			if (!video)
				return false;
			const GUID& wanted = codec == SystemCodec::kH264 ? kH264DecodeProfile : kAv1DecodeProfile;
			for (UINT index = 0, count = video->GetVideoDecoderProfileCount(); index < count; ++index)
			{
				GUID profile{};
				if (SUCCEEDED(video->GetVideoDecoderProfile(index, &profile)) && profile == wanted)
				{
					BOOL supported = FALSE;
					return SUCCEEDED(video->CheckVideoDecoderFormat(&profile, DXGI_FORMAT_NV12, &supported)) &&
					       supported;
				}
			}
			return false;
		}

		// What the system has, in the order the encoder picks.
		MfCodecs Look(SystemCodec codec)
		{
			MfCodecs found;
			if (auto hardware = HardwareEncoders(codec); !hardware.empty())
			{
				found.encoder         = Name(hardware.front().get(), codec);
				found.hardwareEncoder = true;
			}
			else if (auto software = SoftwareEncoders(codec); !software.empty())
				found.encoder = Name(software.front().get(), codec);
			if (auto decoders = Decoders(codec); !decoders.empty())
			{
				found.decoder         = Name(decoders.front().get(), codec);
				found.hardwareDecoder = GpuDecodes(codec);
			}
			RTC_LOG(LS_INFO) << "Media Foundation " << CodecName(codec) << ": encoder \"" << found.encoder << "\""
			                 << (found.hardwareEncoder ? " (hardware)" : "") << ", decoder \"" << found.decoder
			                 << "\"" << (found.hardwareDecoder ? " (hardware)" : "");
			return found;
		}

		// On a thread of its own: EnsureMf joins the calling thread to COM's
		// multithreaded apartment, which an app's UI thread must not be in.
		MfCodecs LookOnce(SystemCodec codec)
		{
			MfCodecs found;
			std::thread([&found, codec] {
				if (EnsureMf())
					found = Look(codec);
			}).join();
			return found;
		}
	} // namespace

	const MfCodecs& FindMfCodecs(SystemCodec codec)
	{
		static const MfCodecs h264 = LookOnce(SystemCodec::kH264);
		static const MfCodecs av1  = LookOnce(SystemCodec::kAv1);
		return codec == SystemCodec::kH264 ? h264 : av1;
	}

	std::unique_ptr<webrtc::VideoEncoder> CreateMfEncoder(SystemCodec codec)
	{
		if (FindMfCodecs(codec).encoder.empty())
			return nullptr;
		if (codec == SystemCodec::kH264)
			return std::make_unique<LayeredEncoder>();
		return std::make_unique<MfEncoder>(codec);
	}

	std::unique_ptr<webrtc::VideoDecoder> CreateMfDecoder(SystemCodec codec)
	{
		if (FindMfCodecs(codec).decoder.empty())
			return nullptr;
		return std::make_unique<MfDecoder>(codec);
	}
} // namespace gelabber
