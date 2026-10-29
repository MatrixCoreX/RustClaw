"""HTTP transport for credential-bearing operator API calls, not web browsing."""
import urllib.request


class _RejectRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def authenticated_opener(*, disable_proxy=False):
    handlers = [_RejectRedirects()]
    if disable_proxy:
        handlers.insert(0, urllib.request.ProxyHandler({}))
    return urllib.request.build_opener(*handlers)


def authenticated_urlopen(request, *, timeout):
    return authenticated_opener().open(request, timeout=timeout)
