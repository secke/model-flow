"""Webhook receiver to try alerts locally: prints every alert it receives.
Usage: python3 tools/webhook-echo.py [port]   (default 9000); URL: http://127.0.0.1:9000/"""
import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        try:
            print("ALERT:", json.loads(body).get("text"), flush=True)
        except ValueError:
            print("ALERT (raw):", body[:500], flush=True)
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"ok")

    def log_message(self, *args):
        pass

port = int(sys.argv[1]) if len(sys.argv) > 1 else 9000
print(f"Waiting for alerts on http://127.0.0.1:{port}/ (Ctrl+C to stop)", flush=True)
HTTPServer(("127.0.0.1", port), Handler).serve_forever()
