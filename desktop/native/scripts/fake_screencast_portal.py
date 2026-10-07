#!/usr/bin/python3
"""Test-only xdg-desktop-portal ScreenCast backend.

Answers the real xdg-desktop-portal frontend like a desktop whose user picked
one monitor without a dialog: Start returns a fixed PipeWire node (a GStreamer
test video in CI). Everything a client sees still goes through the frontend:
the D-Bus requests, OpenPipeWireRemote and the restricted PipeWire connection.

Usage: fake_screencast_portal.py <node-id> <width> <height>
"""

import sys

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

BUS_NAME = "org.freedesktop.impl.portal.desktop.gelabbertest"
PATH = "/org/freedesktop/portal/desktop"

XML = """
<node>
  <interface name="org.freedesktop.impl.portal.ScreenCast">
    <method name="CreateSession">
      <arg type="o" name="handle" direction="in"/>
      <arg type="o" name="session_handle" direction="in"/>
      <arg type="s" name="app_id" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/>
      <arg type="a{sv}" name="results" direction="out"/>
    </method>
    <method name="SelectSources">
      <arg type="o" name="handle" direction="in"/>
      <arg type="o" name="session_handle" direction="in"/>
      <arg type="s" name="app_id" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/>
      <arg type="a{sv}" name="results" direction="out"/>
    </method>
    <method name="Start">
      <arg type="o" name="handle" direction="in"/>
      <arg type="o" name="session_handle" direction="in"/>
      <arg type="s" name="app_id" direction="in"/>
      <arg type="s" name="parent_window" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/>
      <arg type="a{sv}" name="results" direction="out"/>
    </method>
    <property name="AvailableSourceTypes" type="u" access="read"/>
    <property name="AvailableCursorModes" type="u" access="read"/>
    <property name="version" type="u" access="read"/>
  </interface>
</node>
"""

SESSION_XML = """
<node>
  <interface name="org.freedesktop.impl.portal.Session">
    <method name="Close"/>
    <signal name="Closed"/>
    <property name="version" type="u" access="read"/>
  </interface>
</node>
"""


def main():
    node_id, width, height = (int(arg) for arg in sys.argv[1:4])
    info = Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0]
    session_info = Gio.DBusNodeInfo.new_for_xml(SESSION_XML).interfaces[0]
    sessions = {}

    def log(*args):
        print("fake portal:", *args, file=sys.stderr, flush=True)

    def session_call(connection, sender, path, interface, method, params, invocation):
        log("session", method, path)
        registration = sessions.pop(path, None)
        if registration is not None:
            connection.unregister_object(registration)
        invocation.return_value(None)

    def session_property(connection, sender, path, interface, name):
        return GLib.Variant("u", 1)

    def call(connection, sender, path, interface, method, params, invocation):
        log(method, params.unpack()[:2])
        results = {}
        if method == "CreateSession":
            session = params.unpack()[1]
            sessions[session] = connection.register_object(
                session, session_info, session_call, session_property, None
            )
            results["session_id"] = GLib.Variant("s", session.rsplit("/", 1)[-1])
        elif method == "Start":
            stream = (
                node_id,
                {
                    "size": GLib.Variant("(ii)", (width, height)),
                    "position": GLib.Variant("(ii)", (0, 0)),
                    "source_type": GLib.Variant("u", 1),
                },
            )
            results["streams"] = GLib.Variant("a(ua{sv})", [stream])
            results["persist_mode"] = GLib.Variant("u", 0)
        invocation.return_value(GLib.Variant("(ua{sv})", (0, results)))

    def get_property(connection, sender, path, interface, name):
        return GLib.Variant("u", {"AvailableSourceTypes": 7, "AvailableCursorModes": 7, "version": 5}[name])

    def acquired(connection, name):
        connection.register_object(PATH, info, call, get_property, None)
        log("serving node", node_id, f"{width}x{height}")

    Gio.bus_own_name(
        Gio.BusType.SESSION,
        BUS_NAME,
        Gio.BusNameOwnerFlags.NONE,
        acquired,
        None,
        lambda *_: sys.exit("fake portal: lost bus name"),
    )
    GLib.MainLoop().run()


if __name__ == "__main__":
    main()
