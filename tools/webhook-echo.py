"""Récepteur de webhook pour essayer les alertes en local : affiche chaque alerte reçue.
Usage : python3 tools/webhook-echo.py [port]   (défaut 9000) ; URL : http://127.0.0.1:9000/"""
import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        try:
            print("ALERTE :", json.loads(body).get("text"), flush=True)
        except ValueError:
            print("ALERTE (brute) :", body[:500], flush=True)
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"ok")

    def log_message(self, *args):
        pass

port = int(sys.argv[1]) if len(sys.argv) > 1 else 9000
print(f"En attente d'alertes sur http://127.0.0.1:{port}/ (Ctrl+C pour arrêter)", flush=True)
HTTPServer(("127.0.0.1", port), Handler).serve_forever()
