// Lecteur d'affichage (MVP). Modes, via le fragment d'URL :
//   #token=<jeton_écran>   écran réel (marque l'écran "connecté")
//   #screen=<id>           ce que voit un écran (jeton admin, ne marque rien)
//   #version=<id>          une version précise (jeton admin)
//   #playlist=<id>         le brouillon courant d'une playlist (jeton admin)
(() => {
  const params = new URLSearchParams(location.hash.slice(1));
  const stage = document.getElementById("stage");
  const badge = document.getElementById("badge");
  const POLL_MS = 10000;

  let source;
  const adminUrl = params.get("screen") ? `/api/screens/${params.get("screen")}/manifest`
    : params.get("version") ? `/api/versions/${params.get("version")}`
    : params.get("playlist") ? `/api/playlists/${params.get("playlist")}/preview` : null;
  if (params.get("token")) {
    source = { url: "/api/player/manifest", headers: { Authorization: "Bearer " + params.get("token") }, preview: false };
  } else if (adminUrl) {
    let admin = sessionStorage.getItem("signage_admin");
    if (!admin) {
      admin = prompt("Jeton admin (prévisualisation) :") || "";
      sessionStorage.setItem("signage_admin", admin);
    }
    source = { url: adminUrl, headers: { Authorization: "Bearer " + admin }, preview: true };
  } else {
    stage.innerHTML = '<div class="msg">Ajoutez <b>#token=…</b>, <b>#screen=1</b>, <b>#version=1</b> ou <b>#playlist=1</b> à l\'adresse.</div>';
    return;
  }

  let list = [], idx = -1, version = null, key = null, pending = null, pendingKey = null, timer = null;

  function setBadge(text) {
    if (!source.preview) return;
    badge.textContent = text;
    badge.classList.add("on");
  }
  function message(text) {
    const d = document.createElement("div");
    d.className = "msg";
    d.textContent = text;
    stage.replaceChildren(d);
  }

  async function fetchManifest() {
    try {
      const r = await fetch(source.url, { headers: source.headers, cache: "no-store" });
      if (r.status === 401) { if (source.preview) sessionStorage.removeItem("signage_admin"); return { error: "Accès refusé (jeton invalide)" }; }
      if (r.status === 404) return { error: "Aucune version affectée" };
      if (!r.ok) return { error: "Erreur " + r.status };
      return { data: await r.json() };
    } catch (e) {
      return { error: "Serveur injoignable" };
    }
  }

  function show(item) {
    let el;
    if (item.type === "text") {
      el = document.createElement("div");
      el.className = "text";
      el.textContent = item.text;
    } else if (item.type === "image") {
      el = document.createElement("img");
      el.src = item.media.url;
      el.onerror = () => { clearTimeout(timer); timer = setTimeout(next, 2000); };
    } else {
      el = document.createElement("video");
      el.src = item.media.url;
      el.muted = true; el.autoplay = true; el.playsInline = true;
      el.onended = next;
      el.onerror = () => { clearTimeout(timer); timer = setTimeout(next, 2000); };
    }
    stage.replaceChildren(el);
    if (item.duration_s) timer = setTimeout(next, item.duration_s * 1000);
  }

  function next() {
    clearTimeout(timer);
    if (pending) {
      list = pending.items; version = pending.version_id; key = pendingKey; pending = null; idx = -1;
    }
    if (!list.length) { timer = setTimeout(next, 5000); return; }
    idx = (idx + 1) % list.length;
    setBadge(`${version ? "version #" + version : "brouillon"} · élément ${idx + 1}/${list.length}`);
    show(list[idx]);
  }

  async function poll() {
    const res = await fetchManifest();
    if (res.error) {
      if (!list.length) message(res.error);  // sinon on garde la dernière version valide
      return;
    }
    const k = res.data.version_id + ":" + JSON.stringify(res.data.items);
    if (k !== (pending ? pendingKey : key)) { pending = res.data; pendingKey = k; }  // appliquée à la fin de l'élément courant
  }

  document.addEventListener("click", () => document.documentElement.requestFullscreen?.());
  (async () => { await poll(); next(); setInterval(poll, POLL_MS); })();
})();
