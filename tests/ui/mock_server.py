"""Faux backend TTScreen (en mémoire) pour tester l'interface dans un vrai navigateur."""
import json, re, sys, mimetypes, os
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
from datetime import datetime, timedelta

ROOT = sys.argv[1]
PORT = int(sys.argv[2])
NOW = lambda: datetime.utcnow().isoformat()

S = {
    "playlists": [
        {"id": 1, "name": "Promo", "items": [{"id": 1, "type": "text", "text": "SOLDES", "media_id": None, "duration_s": 10, "position": 0}]},
        {"id": 2, "name": "Hall", "items": []},
    ],
    "versions": {1: [{"id": 1, "playlist_id": 1, "number": 1, "created_at": NOW(), "manifest": {"items": []}}], 2: []},
    "media": [], "users": [{"id": 1, "username": "emma", "role": "admin", "active": True, "created_at": NOW(), "last_login": NOW()}],
    "groups": [], "screens": [
        {"id": 1, "name": "Entrée", "version_id": 1, "token": "tok1", "last_seen": NOW(), "group_id": None},
        {"id": 2, "name": "Caisse", "version_id": None, "token": "tok2", "last_seen": None, "group_id": None},
    ],
    "schedules": [], "audit": [], "next": {"playlist": 3, "group": 1, "screen": 3, "rule": 1},
}
VNAME = {1: "Promo"}

def screen_out(s):
    g = next((g for g in S["groups"] if g["id"] == s["group_id"]), None)
    rule = next((r for r in S["schedules"] if r["screen_id"] == s["id"]), None)
    eff, src = None, None
    if rule: eff, src = rule["version_id"], "screen_schedule"
    elif s["version_id"]: eff, src = s["version_id"], "screen"
    elif g and g["version_id"]: eff, src = g["version_id"], "group"
    online = bool(s["last_seen"])
    return {"id": s["id"], "name": s["name"], "version_id": s["version_id"], "online": online, "last_seen": s["last_seen"],
            "has_token": True, "group_id": g["id"] if g else None, "group_name": g["name"] if g else None,
            "effective_version_id": eff, "effective_source": src}

def group_out(g):
    return {"id": g["id"], "name": g["name"], "version_id": g["version_id"],
            "screen_count": sum(1 for s in S["screens"] if s["group_id"] == g["id"])}

class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def send(self, code, obj=None):
        body = b"" if obj is None else json.dumps(obj).encode()
        self.send_response(code)
        if obj is not None: self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers(); self.wfile.write(body)
    def body(self):
        n = int(self.headers.get("Content-Length") or 0)
        return json.loads(self.rfile.read(n) or b"{}") if n else {}
    def static(self, path):
        base = {"/admin/": "frontend", "/player/": "player"}
        for prefix, d in base.items():
            if path == prefix[:-1]: self.send_response(307); self.send_header("Location", prefix); self.end_headers(); return True
            if path.startswith(prefix):
                rel = path[len(prefix):] or "index.html"
                f = os.path.join(ROOT, d, rel)
                if os.path.isfile(f):
                    data = open(f, "rb").read()
                    self.send_response(200); self.send_header("Content-Type", mimetypes.guess_type(f)[0] or "application/octet-stream")
                    self.send_header("Content-Length", str(len(data))); self.end_headers(); self.wfile.write(data); return True
                self.send(404, {"detail": "Not Found"}); return True
        return False
    def route(self, method):
        path = self.path.split("?")[0]
        if method == "GET" and self.static(path): return
        if path == "/api/auth/login": return self.send(200, {"token": "T", "username": "emma", "role": "admin", "expires_at": None})
        if path == "/api/auth/me": return self.send(200, {"username": "emma", "role": "admin", "legacy": False})
        if path == "/api/auth/password":
            b = self.body()
            return self.send(204) if b.get("current_password") == "ancien123" else self.send(403, {"detail": "Mot de passe actuel incorrect"})
        if path == "/api/stats":
            outs = [screen_out(s) for s in S["screens"]]
            on = sum(o["online"] for o in outs)
            return self.send(200, {"screens": {"total": len(outs), "online": on, "offline": len(outs) - on,
                "without_content": sum(1 for o in outs if o["effective_version_id"] is None)},
                "groups": len(S["groups"]), "playlists": len(S["playlists"]), "versions": 1, "schedules": len(S["schedules"]),
                "media": {"count": 0, "bytes": 0}})
        if path == "/api/media": return self.send(200, S["media"])
        if path == "/api/users": return self.send(200, S["users"])
        if path == "/api/audit": return self.send(200, S["audit"])
        m = re.fullmatch(r"/api/playlists/(\d+)/versions", path)
        if m: return self.send(200, S["versions"].get(int(m.group(1)), []))
        m = re.fullmatch(r"/api/playlists/(\d+)/duplicate", path)
        if m and method == "POST":
            src = next(p for p in S["playlists"] if p["id"] == int(m.group(1)))
            pid = S["next"]["playlist"]; S["next"]["playlist"] += 1
            cp = {"id": pid, "name": "Copie de " + src["name"], "items": [dict(i, id=i["id"] + 100) for i in src["items"]]}
            S["playlists"].append(cp); S["versions"][pid] = []; return self.send(201, cp)
        m = re.fullmatch(r"/api/playlists/(\d+)", path)
        if m and method == "GET": return self.send(200, next(p for p in S["playlists"] if p["id"] == int(m.group(1))))
        if path == "/api/playlists": return self.send(200, S["playlists"])
        # groupes
        if path == "/api/groups":
            if method == "GET": return self.send(200, [group_out(g) for g in sorted(S["groups"], key=lambda g: g["name"])])
            b = self.body(); g = {"id": S["next"]["group"], "name": b["name"], "version_id": None}; S["next"]["group"] += 1
            S["groups"].append(g); return self.send(201, group_out(g))
        m = re.fullmatch(r"/api/groups/(\d+)(/assign)?", path)
        if m:
            g = next((g for g in S["groups"] if g["id"] == int(m.group(1))), None)
            if not g: return self.send(404, {"detail": "Groupe introuvable"})
            if method == "DELETE":
                for s in S["screens"]:
                    if s["group_id"] == g["id"]: s["group_id"] = None
                S["groups"].remove(g); return self.send(204)
            b = self.body()
            if m.group(2): g["version_id"] = b["version_id"]
            else: g["name"] = b["name"]
            return self.send(200, group_out(g))
        # planning
        if path == "/api/schedules":
            if method == "GET": return self.send(200, S["schedules"])
            b = self.body(); r = {"id": S["next"]["rule"], "screen_id": b.get("screen_id"), "group_id": b.get("group_id"),
                "version_id": b["version_id"], "days": sorted(b["days"]), "start": b["start"], "end": b["end"], "created_at": NOW()}
            S["next"]["rule"] += 1; S["schedules"].append(r); return self.send(201, r)
        m = re.fullmatch(r"/api/schedules/(\d+)", path)
        if m and method == "DELETE":
            S["schedules"] = [r for r in S["schedules"] if r["id"] != int(m.group(1))]; return self.send(204)
        # écrans
        if path == "/api/screens":
            if method == "GET": return self.send(200, [screen_out(s) for s in S["screens"]])
            b = self.body(); s = {"id": S["next"]["screen"], "name": b["name"], "version_id": None, "token": "newtok", "last_seen": None, "group_id": b.get("group_id")}
            S["next"]["screen"] += 1; S["screens"].append(s); return self.send(201, dict(screen_out(s), token="newtok"))
        m = re.fullmatch(r"/api/screens/(\d+)(/group|/assign|/rotate-token|/revoke-token)?", path)
        if m:
            s = next((s for s in S["screens"] if s["id"] == int(m.group(1))), None)
            if not s: return self.send(404, {"detail": "Écran introuvable"})
            if method == "DELETE": S["screens"].remove(s); return self.send(204)
            b = self.body(); sub = m.group(2)
            if sub == "/group": s["group_id"] = b["group_id"]
            elif sub == "/assign": s["version_id"] = b["version_id"]
            elif sub is None: s["name"] = b["name"]
            return self.send(200, screen_out(s))
        self.send(404, {"detail": "Not Found"})
    def do_GET(self): self.route("GET")
    def do_POST(self): self.route("POST")
    def do_PUT(self): self.route("PUT")
    def do_DELETE(self): self.route("DELETE")

ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
