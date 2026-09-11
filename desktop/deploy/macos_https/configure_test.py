import unittest
from pathlib import Path
from templates import nginx_config, safe_path, validate_network


class HttpsConfigurationTest(unittest.TestCase):
    def test_only_https_is_added_and_forwarded_identity_is_overwritten(self):
        config = nginx_config(Path('/tmp/secure endpoint'), Path('/tmp/existing ui'),
                              Path('/etc/nginx/mime.types'), '192.168.31.162', 'host.local', '192.168.31.0/24')
        self.assertNotIn(':80;', config)
        self.assertNotIn('Strict-Transport-Security', config)
        self.assertNotIn('return 301', config)
        self.assertIn('proxy_set_header X-Forwarded-For $remote_addr;', config)
        self.assertIn('proxy_set_header Host $http_host;', config)
        self.assertIn('deny all;', config)
        self.assertIn('ssl_protocols TLSv1.2 TLSv1.3;', config)

    def test_public_addresses_wide_networks_and_config_injection_are_rejected(self):
        for address, host, subnet in [('8.8.8.8', 'host.local', '8.8.8.0/24'),
                                      ('192.168.31.162', 'host.local', '192.168.0.0/16'),
                                      ('192.168.31.162', 'evil;include.local', '192.168.31.0/24')]:
            with self.assertRaises(ValueError):
                validate_network(address, host, subnet)
        for path in ['/tmp/";include x;', '/tmp/$config', '/tmp/line\nbreak', 'relative']:
            with self.assertRaises(ValueError):
                safe_path(path)


if __name__ == '__main__':
    unittest.main()
