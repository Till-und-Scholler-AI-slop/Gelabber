#include "gst_common.h"

#include <rtc_base/logging.h>

#include <dlfcn.h>

namespace gelabber::gst
{
	namespace
	{
		template<typename F>
		bool resolve(void* lib, const char* name, F*& out)
		{
			out = reinterpret_cast<F*>(dlsym(lib, name));
			return out != nullptr;
		}
	} // namespace

	const Api& Gst()
	{
		static const Api api = [] {
			Api api{};
			void* core  = dlopen("libgstreamer-1.0.so.0", RTLD_NOW | RTLD_GLOBAL);
			void* app   = core ? dlopen("libgstapp-1.0.so.0", RTLD_NOW | RTLD_GLOBAL) : nullptr;
			void* video = core ? dlopen("libgstvideo-1.0.so.0", RTLD_NOW | RTLD_GLOBAL) : nullptr;
			if (!core || !app || !video)
			{
				RTC_LOG(LS_INFO) << "GStreamer not available";
				return api;
			}
			api.ok = resolve(core, "gst_init_check", api.init_check) &&
			         resolve(core, "gst_parse_launch", api.parse_launch) &&
			         resolve(core, "gst_bin_get_by_name", api.bin_get_by_name) &&
			         resolve(core, "gst_element_set_state", api.element_set_state) &&
			         resolve(core, "gst_element_get_bus", api.element_get_bus) &&
			         resolve(core, "gst_element_send_event", api.element_send_event) &&
			         resolve(core, "gst_element_factory_find", api.element_factory_find) &&
			         resolve(core, "gst_object_unref", api.object_unref) &&
			         resolve(core, "gst_bus_pop_filtered", api.bus_pop_filtered) &&
			         resolve(core, "gst_message_parse_error", api.message_parse_error) &&
			         resolve(core, "gst_mini_object_unref", api.mini_object_unref) &&
			         resolve(core, "gst_buffer_new_allocate", api.buffer_new_allocate) &&
			         resolve(core, "gst_buffer_map", api.buffer_map) &&
			         resolve(core, "gst_buffer_unmap", api.buffer_unmap) &&
			         resolve(core, "gst_sample_get_buffer", api.sample_get_buffer) &&
			         resolve(core, "gst_sample_get_caps", api.sample_get_caps) &&
			         resolve(core, "gst_caps_get_structure", api.caps_get_structure) &&
			         resolve(core, "gst_structure_get_int", api.structure_get_int) &&
			         resolve(core, "gst_util_set_object_arg", api.util_set_object_arg) &&
			         resolve(app, "gst_app_src_push_buffer", api.app_src_push_buffer) &&
			         resolve(app, "gst_app_sink_try_pull_sample", api.app_sink_try_pull_sample) &&
			         resolve(
			           video,
			           "gst_video_event_new_upstream_force_key_unit",
			           api.video_event_new_upstream_force_key_unit) &&
			         resolve(video, "gst_buffer_get_video_meta", api.buffer_get_video_meta);
			if (!api.ok)
			{
				RTC_LOG(LS_WARNING) << "GStreamer is missing symbols";
				return api;
			}
			GError* error = nullptr;
			if (!api.init_check(nullptr, nullptr, &error))
			{
				RTC_LOG(LS_WARNING) << "gst_init failed: " << (error ? error->message : "?");
				if (error)
					g_error_free(error);
				api.ok = false;
			}
			return api;
		}();
		return api;
	}

	bool HasElement(const std::string& name)
	{
		const auto& gst = Gst();
		if (!gst.ok)
			return false;
		void* factory = gst.element_factory_find(name.c_str());
		if (!factory)
			return false;
		gst.object_unref(factory);
		return true;
	}

	bool SetArg(GstElement* element, const char* name, const char* value)
	{
		auto* object      = G_OBJECT(element);
		GParamSpec* param = g_object_class_find_property(G_OBJECT_GET_CLASS(object), name);
		if (!param)
			return false;
		if (G_IS_PARAM_SPEC_ENUM(param))
		{
			auto* enumClass = G_PARAM_SPEC_ENUM(param)->enum_class;
			if (!g_enum_get_value_by_nick(enumClass, value) && !g_enum_get_value_by_name(enumClass, value))
				return false;
		}
		Gst().util_set_object_arg(object, name, value);
		return true;
	}

	void SetFirst(GstElement* element, const char* name, std::initializer_list<const char*> values)
	{
		for (const char* value : values)
			if (SetArg(element, name, value))
				return;
	}

	bool PopErrors(GstBus* bus, const std::string& element)
	{
		const auto& gst = Gst();
		bool error      = false;
		while (GstMessage* message = gst.bus_pop_filtered(bus, kMessageError))
		{
			GError* gerror = nullptr;
			char* debug    = nullptr;
			gst.message_parse_error(message, &gerror, &debug);
			RTC_LOG(LS_WARNING) << "GStreamer " << element << ": " << (gerror ? gerror->message : "error")
			                    << " " << (debug ? debug : "");
			if (gerror)
				g_error_free(gerror);
			g_free(debug);
			gst.mini_object_unref(message);
			error = true;
		}
		return error;
	}
} // namespace gelabber::gst
