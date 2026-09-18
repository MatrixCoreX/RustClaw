"""Close a GTK toplevel on the test's isolated X display, as its titlebar does.

WebKitWebDriver DELETE /window destroys only the embedded WebView in Wry;
it does not exercise the native window's CloseRequested/Destroyed events.
"""
import ctypes as c


def close_security_window():
    x = c.CDLL('libX11.so.6')
    window = c.c_ulong
    display = c.c_void_p
    x.XOpenDisplay.argtypes, x.XOpenDisplay.restype = [c.c_char_p], display
    x.XDefaultRootWindow.argtypes, x.XDefaultRootWindow.restype = [display], window
    x.XQueryTree.argtypes = [display, window, c.POINTER(window), c.POINTER(window), c.POINTER(c.POINTER(window)), c.POINTER(c.c_uint)]
    x.XFetchName.argtypes = [display, window, c.POINTER(c.c_char_p)]
    x.XFree.argtypes = [c.c_void_p]
    x.XInternAtom.argtypes, x.XInternAtom.restype = [display, c.c_char_p, c.c_int], c.c_ulong
    x.XGetWindowProperty.argtypes = [display, window, c.c_ulong, c.c_long, c.c_long, c.c_int, c.c_ulong,
                                    c.POINTER(c.c_ulong), c.POINTER(c.c_int), c.POINTER(c.c_ulong),
                                    c.POINTER(c.c_ulong), c.POINTER(c.c_void_p)]
    x.XCloseDisplay.argtypes = [display]

    class Data(c.Union):
        _fields_ = [('b', c.c_char * 20), ('s', c.c_short * 10), ('l', c.c_long * 5)]

    class ClientMessage(c.Structure):
        _fields_ = [('type', c.c_int), ('serial', c.c_ulong), ('send_event', c.c_int),
                    ('display', display), ('window', window), ('message_type', c.c_ulong),
                    ('format', c.c_int), ('data', Data)]

    class Event(c.Union):
        _fields_ = [('client', ClientMessage), ('pad', c.c_long * 24)]

    x.XSendEvent.argtypes = [display, window, c.c_int, c.c_long, c.POINTER(Event)]
    x.XSync.argtypes = [display, c.c_int]
    d = x.XOpenDisplay(None)
    assert d, 'isolated test X display unavailable'
    try:
        root, parent, children, count = window(), window(), c.POINTER(window)(), c.c_uint()
        assert x.XQueryTree(d, x.XDefaultRootWindow(d), c.byref(root), c.byref(parent), c.byref(children), c.byref(count))
        matches, titles = [], []
        try:
            for i in range(count.value):
                value, atom, size, length, remaining = c.c_void_p(), c.c_ulong(), c.c_int(), c.c_ulong(), c.c_ulong()
                status = x.XGetWindowProperty(d, children[i], x.XInternAtom(d, b'_NET_WM_NAME', 0),
                                             0, 2048, 0, 0, c.byref(atom), c.byref(size), c.byref(length),
                                             c.byref(remaining), c.byref(value))
                if status == 0 and value.value:
                    title = c.string_at(value, length.value).decode('utf-8', 'replace')
                    titles.append(title)
                    if title.endswith(' · 本地资产安全窗口'):
                        matches.append(children[i])
                    x.XFree(value)
        finally:
            if children:
                x.XFree(children)
        assert len(matches) == 1, ('expected exactly one native security window', titles)
        event = Event()
        event.client.type = 33  # ClientMessage
        event.client.send_event = 1
        event.client.display = d
        event.client.window = matches[0]
        event.client.message_type = x.XInternAtom(d, b'WM_PROTOCOLS', 0)
        event.client.format = 32
        event.client.data.l[0] = x.XInternAtom(d, b'WM_DELETE_WINDOW', 0)
        assert x.XSendEvent(d, matches[0], 0, 0, c.byref(event))
        x.XSync(d, 0)
    finally:
        x.XCloseDisplay(d)
