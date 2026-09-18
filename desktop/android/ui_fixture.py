"""HTTPS fixtures reached through the emulator's loopback-host alias."""
import subprocess
from fixture_server import Fixture


class AndroidFixture(Fixture):
    def __init__(self, directory):
        super().__init__(directory)
        # Keep servers bound to host loopback. Direct emulator traffic avoids
        # routing all console requests through adb's test-control connection.
        extension=directory/'android-extensions.cnf'
        extension.write_text('subjectAltName=DNS:localhost,IP:127.0.0.1,IP:10.0.2.2\nbasicConstraints=critical,CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n')
        subprocess.run(['openssl','x509','-req','-in',str(directory/'leaf.csr'),
                        '-CA',str(directory/'ca.pem'),'-CAkey',str(directory/'ca.key'),
                        '-CAcreateserial','-out',str(directory/'leaf.pem'),'-days','1','-sha256',
                        '-extfile',str(extension)],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        self.server.socket.context.load_cert_chain(directory/'leaf.pem',directory/'leaf.key')
        self.origin=f'https://10.0.2.2:{self.server.server_port}'
