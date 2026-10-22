"""Serves sign-in.html over HTTPS, for tools/ios-autofill-test.sh: Safari on the
simulator trusts the certificate because the script adds its throwaway
authority to the simulator. Usage: serve.py <port> <cert.pem> <key.pem>."""

import http.server
import os
import ssl
import sys

port, cert, key = int(sys.argv[1]), sys.argv[2], sys.argv[3]
os.chdir(os.path.dirname(os.path.abspath(__file__)))


class Page(http.server.SimpleHTTPRequestHandler):
    def do_GET(self):
        self.path = "/sign-in.html"
        super().do_GET()

    def log_message(self, *args):
        pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", port), Page)
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(cert, key)
server.socket = context.wrap_socket(server.socket, server_side=True)
server.serve_forever()
