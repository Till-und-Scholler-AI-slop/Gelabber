// Turns libmediasoupclient's transport callbacks into numbered events for
// the embedder (gm_event_fn), answered through gm_transport_respond.
// Header-only, included by gelabber_media.cc.
#ifndef GELABBER_TRANSPORT_BRIDGE_H
#define GELABBER_TRANSPORT_BRIDGE_H

#include "gelabber_media.h"

#include "mediasoupclient.hpp"

#include <cstdint>
#include <exception>
#include <future>
#include <map>
#include <mutex>
#include <stdexcept>
#include <string>

namespace gelabber
{
	using json = nlohmann::json;

	// Shared by send and receive listeners: turns libmediasoupclient's
	// blocking std::future callbacks into numbered events answered through
	// gm_transport_respond.
	class Bridge
	{
	public:
		Bridge(gm_event_fn onEvent, void* user) : onEvent(onEvent), user(user)
		{
		}

		std::future<void> Connect(const json& dtlsParameters)
		{
			std::promise<void> promise;
			auto future = promise.get_future();
			uint64_t id;
			{
				std::lock_guard<std::mutex> lock(mutex);
				id = ++nextRequest;
				connects.emplace(id, std::move(promise));
			}
			Emit(GM_EVENT_CONNECT, id, dtlsParameters);
			return future;
		}

		std::future<std::string> Produce(const std::string& kind, const json& rtpParameters, const json& appData)
		{
			std::promise<std::string> promise;
			auto future = promise.get_future();
			uint64_t id;
			{
				std::lock_guard<std::mutex> lock(mutex);
				id = ++nextRequest;
				produces.emplace(id, std::move(promise));
			}
			Emit(
			  GM_EVENT_PRODUCE,
			  id,
			  json{ { "kind", kind }, { "rtpParameters", rtpParameters }, { "appData", appData } });
			return future;
		}

		void State(const std::string& state)
		{
			Emit(GM_EVENT_CONNECTION_STATE, 0, json{ { "state", state } });
		}

		bool Respond(uint64_t request, const char* resultJson, const char* error)
		{
			std::lock_guard<std::mutex> lock(mutex);
			if (auto it = connects.find(request); it != connects.end())
			{
				if (error)
					it->second.set_exception(std::make_exception_ptr(std::runtime_error(error)));
				else
					it->second.set_value();
				connects.erase(it);
				return true;
			}
			if (auto it = produces.find(request); it != produces.end())
			{
				if (error)
				{
					it->second.set_exception(std::make_exception_ptr(std::runtime_error(error)));
				}
				else
				{
					auto result = json::parse(resultJson);
					if (!result.contains("id") || !result["id"].is_string())
						it->second.set_exception(
						  std::make_exception_ptr(std::runtime_error("produce result without id")));
					else
						it->second.set_value(result["id"].get<std::string>());
				}
				produces.erase(it);
				return true;
			}
			return false;
		}

		// Unblocks producers/consumers still waiting when the transport goes away.
		void FailPending()
		{
			std::lock_guard<std::mutex> lock(mutex);
			for (auto& [id, promise] : connects)
				promise.set_exception(std::make_exception_ptr(std::runtime_error("transport closed")));
			for (auto& [id, promise] : produces)
				promise.set_exception(std::make_exception_ptr(std::runtime_error("transport closed")));
			connects.clear();
			produces.clear();
		}

	private:
		void Emit(gm_event_kind kind, uint64_t request, const json& payload)
		{
			if (onEvent)
			{
				const auto text = payload.dump();
				onEvent(user, kind, request, text.c_str());
			}
		}

		gm_event_fn onEvent;
		void* user;
		std::mutex mutex;
		uint64_t nextRequest{ 0 };
		std::map<uint64_t, std::promise<void>> connects;
		std::map<uint64_t, std::promise<std::string>> produces;
	};

	class SendListener : public mediasoupclient::SendTransport::Listener
	{
	public:
		explicit SendListener(Bridge* bridge) : bridge(bridge)
		{
		}
		std::future<void> OnConnect(mediasoupclient::Transport*, const json& dtlsParameters) override
		{
			return bridge->Connect(dtlsParameters);
		}
		void OnConnectionStateChange(mediasoupclient::Transport*, const std::string& state) override
		{
			bridge->State(state);
		}
		std::future<std::string> OnProduce(
		  mediasoupclient::SendTransport*, const std::string& kind, json rtpParameters, const json& appData) override
		{
			return bridge->Produce(kind, rtpParameters, appData);
		}
		std::future<std::string> OnProduceData(
		  mediasoupclient::SendTransport*, const json&, const std::string&, const std::string&, const json&) override
		{
			std::promise<std::string> promise;
			promise.set_exception(std::make_exception_ptr(std::runtime_error("data channels unsupported")));
			return promise.get_future();
		}

	private:
		Bridge* bridge;
	};

	class RecvListener : public mediasoupclient::RecvTransport::Listener
	{
	public:
		explicit RecvListener(Bridge* bridge) : bridge(bridge)
		{
		}
		std::future<void> OnConnect(mediasoupclient::Transport*, const json& dtlsParameters) override
		{
			return bridge->Connect(dtlsParameters);
		}
		void OnConnectionStateChange(mediasoupclient::Transport*, const std::string& state) override
		{
			bridge->State(state);
		}

	private:
		Bridge* bridge;
	};
} // namespace gelabber

#endif
