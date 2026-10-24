"""A stand-in for GitHub's releases on this computer, for
tools/desktop-update-test.sh: the list of releases, and each release's
files, as GitHub answers (the API's list, and a file's address redirecting
to where it is). Serves the folder it's given:

  releases.json         the list (GitHub's /repos/<repo>/releases)
  <tag>/<file name>     each release's files

and logs each request to requests.log there. Writes the port it listens
on (on 127.0.0.1 only) to the file named second.

  python3 tools/github-stand-in.py <folder> <port file>
"""

import http.server
import os
import sys

REPO = "PimpinPumpkin/passbundle-apps"
ROOT = os.path.abspath(sys.argv[1])


class StandIn(http.server.BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        with open(os.path.join(ROOT, "requests.log"), "a", encoding="utf-8") as log:
            log.write(f"{self.command} {self.path}\n")

    def send(self, status, body=b"", headers=()):
        self.send_response(status)
        for name, value in headers:
            self.send_header(name, value)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def file(self, *parts):
        path = os.path.abspath(os.path.join(ROOT, *parts))
        if not path.startswith(ROOT + os.sep) or not os.path.isfile(path):
            return None
        with open(path, "rb") as f:
            return f.read()

    def do_GET(self):
        downloads = f"/{REPO}/releases/download/"
        if self.path == f"/repos/{REPO}/releases?per_page=100":
            body = self.file("releases.json")
            return self.send(200, body) if body is not None else self.send(404)
        if self.path.startswith(downloads):
            # As github.com hands a file to release-assets.githubusercontent.com.
            rest = self.path[len(downloads):]
            port = self.server.server_address[1]
            return self.send(302, headers=[("Location", f"http://127.0.0.1:{port}/assets/{rest}")])
        if self.path.startswith("/assets/"):
            parts = self.path[len("/assets/"):].split("/")
            body = self.file(*parts) if len(parts) == 2 else None
            return self.send(200, body) if body is not None else self.send(404)
        self.send(404)


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), StandIn)
with open(sys.argv[2], "w", encoding="utf-8") as f:
    f.write(str(server.server_address[1]))
server.serve_forever()
