"""Disposable-session file chooser fixture. Simulates path choice, never a production IPC."""
import threading
from gi.repository import Gio, GLib

XML = '''<node><interface name="org.freedesktop.portal.FileChooser">
<property name="version" type="u" access="read"/>
<method name="SaveFile"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method>
<method name="OpenFile"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method>
</interface></node>'''

class FilePortal:
    def __init__(self, backup_path):
        self.path = backup_path
        self.calls = []
        self.connection = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        self.owner = Gio.bus_own_name_on_connection(self.connection, 'org.freedesktop.portal.Desktop', Gio.BusNameOwnerFlags.NONE, None, None)
        interface = Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0]
        self.registration = self.connection.register_object('/org/freedesktop/portal/desktop', interface, self.method, lambda *_: GLib.Variant('u', 4), None)
        self.loop = GLib.MainLoop()
        threading.Thread(target=self.loop.run, daemon=True).start()

    def method(self, connection, sender, object_path, interface, method, parameters, invocation):
        self.calls.append(method)
        options = parameters.unpack()[2]
        path = '/org/freedesktop/portal/desktop/request/' + sender[1:].replace('.', '_') + '/' + options['handle_token']
        invocation.return_value(GLib.Variant('(o)', (path,)))
        def respond():
            connection.emit_signal(sender, path, 'org.freedesktop.portal.Request', 'Response',
                GLib.Variant('(ua{sv})', (0, {'uris': GLib.Variant('as', [self.path.as_uri()])})))
            return False
        GLib.timeout_add(100, respond)
