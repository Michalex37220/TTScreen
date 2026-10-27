// Interface d'administration (sans étape de compilation). Aucune donnée n'est injectée via innerHTML.
(() => {
"use strict";

const state = {
  token: sessionStorage.getItem("signage_admin") || "",
  view: "playlists",
  playlists: [], media: [], screens: [], versions: {},
  selectedPid: null, draft: [], dirty: false, previewSrc: null,
};
const app = document.getElementById("app");

// ---------- utilitaires ----------
function h(tag, attrs = {}, ...kids) {
  const el = document.createElement(tag);
  let value;
  for (const [k, v] of Object.entries(attrs)) {
    if (v === null || v === undefined || v === false) continue;
    if (k === "class") el.className = v;
    else if (k === "value") value = v;
    else if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else el.setAttribute(k, v === true ? "" : v);
  }
  for (const kid of kids.flat()) {
    if (kid === null || kid === undefined || kid === false) continue;
    el.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
  }
  if (value !== undefined) el.value = value;  // après les <option> pour les <select>
  return el;
}
const $ = (s) => document.querySelector(s);
const act = (fn) => async (...a) => { try { await fn(...a); } catch (e) { toast(e.message || String(e), true); } };

function toast(msg, bad = false) {
  const d = h("div", { class: bad ? "bad" : "" }, msg);
  $("#toast").append(d);
  setTimeout(() => d.remove(), bad ? 6000 : 3500);
}
function fmtSize(n) { return n > 1048576 ? (n / 1048576).toFixed(1) + " Mo" : Math.max(1, Math.round(n / 1024)) + " Ko"; }
function fmtDate(s) { return s ? new Date(s + "Z").toLocaleString("fr-FR") : "jamais"; }
function fmtErr(d, status) {
  if (!d) return "Erreur " + status;
  if (typeof d.detail === "string") return d.detail;
  if (Array.isArray(d.detail)) return d.detail.map((e) => e.msg).join("; ");
  return "Erreur " + status;
}
async function copy(text) {
  try { await navigator.clipboard.writeText(text); toast("Copié"); }
  catch { toast("Copie impossible : sélectionnez le texte à la main", true); }
}
function openModal(title, ...content) {
  const close = () => overlay.remove();
  const overlay = h("div", { class: "overlay", onclick: (e) => { if (e.target === overlay) close(); } },
    h("div", { class: "modal" },
      h("div", { class: "modal-head" }, h("strong", {}, title), h("button", { class: "ghost", onclick: close }, "✕")),
      ...content));
  document.body.append(overlay);
  return close;
}

async function api(path, opts = {}) {
  const headers = { Authorization: "Bearer " + state.token };
  let body = opts.body;
  if (body !== undefined && !(body instanceof FormData)) { headers["Content-Type"] = "application/json"; body = JSON.stringify(body); }
  const r = await fetch(path, { method: opts.method || "GET", headers, body });
  if (r.status === 401) { logout(); throw new Error("Session expirée"); }
  if (r.status === 204) return null;
  const data = await r.json().catch(() => null);
  if (!r.ok) throw new Error(fmtErr(data, r.status));
  return data;
}

const mediaById = (id) => state.media.find((m) => m.id === id);
const toDraft = (it) => ({ type: it.type, text: it.text, media_id: it.media_id, duration_s: it.duration_s });
const loadPlaylists = async () => { state.playlists = await api("/api/playlists"); };
const loadMedia = async () => { state.media = await api("/api/media"); };
const loadScreens = async () => { state.screens = await api("/api/screens"); };
const loadVersions = async () => {
  const entries = await Promise.all(state.playlists.map(async (p) => [p.id, await api(`/api/playlists/${p.id}/versions`)]));
  state.versions = Object.fromEntries(entries);
};
const currentPlaylist = () => state.playlists.find((p) => p.id === state.selectedPid);
const allVersions = () => state.playlists.flatMap((p) => (state.versions[p.id] || []).map((v) => ({ ...v, pname: p.name })));
const versionLabel = (id) => { const v = allVersions().find((x) => x.id === id); return v ? `${v.pname} · v${v.number}` : "—"; };

// ---------- connexion ----------
function logout() {
  sessionStorage.removeItem("signage_admin");
  state.token = "";
  renderLogin();
}
function renderLogin(message = "") {
  const input = h("input", { type: "password", placeholder: "Jeton admin", autofocus: true });
  const err = h("div", { class: "err" }, message);
  const go = async () => {
    const r = await fetch("/api/playlists", { headers: { Authorization: "Bearer " + input.value } });
    if (!r.ok) { err.textContent = "Jeton invalide"; return; }
    state.token = input.value;
    sessionStorage.setItem("signage_admin", state.token);  // lu aussi par le lecteur de prévisualisation
    boot();
  };
  input.addEventListener("keydown", (e) => { if (e.key === "Enter") go(); });
  app.replaceChildren(h("div", { class: "login" },
    h("h1", {}, "TTscreen"),
    h("div", { class: "muted" }, "Jeton admin (variable SIGNAGE_ADMIN_TOKEN du serveur)"),
    input, h("button", { class: "primary", onclick: go }, "Se connecter"), err));
}

// ---------- coque ----------
const boot = act(async () => {
  await Promise.all([loadPlaylists(), loadMedia(), loadScreens()]);
  await loadVersions();
  const tabs = [["playlists", "Playlists"], ["media", "Médias"], ["screens", "Écrans"]];
  app.replaceChildren(
    h("header", {}, h("strong", {}, "TTscreen"),
      h("nav", {}, tabs.map(([k, label]) => h("button", { "data-view": k, onclick: () => setView(k) }, label))),
      h("span", { class: "spacer" }), h("button", { class: "ghost", onclick: logout }, "Déconnexion")),
    h("main", { id: "main" }));
  render();
});

const setView = act(async (v) => {
  if (state.dirty && !confirm("Abandonner les modifications non enregistrées ?")) return;
  state.dirty = false;
  if (v === "screens") await Promise.all([loadScreens(), loadVersions()]);
  if (v === "media") await loadMedia();
  state.view = v;
  render();
});

function render() {
  document.querySelectorAll("nav button").forEach((b) => b.classList.toggle("active", b.dataset.view === state.view));
  const main = $("#main");
  main.replaceChildren();
  ({ playlists: renderPlaylists, media: renderMedia, screens: renderScreens })[state.view](main);
}

// ---------- médias ----------
function thumb(m) {
  if (m.kind === "image") return h("div", { class: "thumb" }, h("img", { src: m.url, loading: "lazy", alt: "" }));
  return h("div", { class: "thumb" }, h("video", { src: m.url + "#t=0.1", preload: "metadata", muted: true }));
}
const uploadFiles = async (files) => {
  let ok = 0;
  for (const [i, f] of files.entries()) {
    toast(`Envoi ${i + 1}/${files.length} : ${f.name}`);
    const fd = new FormData();
    fd.append("file", f);
    try { await api("/api/media", { method: "POST", body: fd }); ok++; }
    catch (e) { toast(`${f.name} : ${e.message}`, true); }
  }
  await loadMedia();
  if (ok) toast(`${ok} fichier(s) ajouté(s)`);
  render();
};
function renderMedia(root) {
  const input = h("input", { type: "file", multiple: true, hidden: true,
    accept: "image/png,image/jpeg,image/webp,image/gif,video/mp4,video/webm",
    onchange: act(async () => { const f = [...input.files]; input.value = ""; await uploadFiles(f); }) });
  const drop = h("div", { class: "drop",
    onclick: () => input.click(),
    ondragover: (e) => { e.preventDefault(); drop.classList.add("over"); },
    ondragleave: () => drop.classList.remove("over"),
    ondrop: act(async (e) => { e.preventDefault(); drop.classList.remove("over"); await uploadFiles([...e.dataTransfer.files]); }),
  }, "Glissez des images ou vidéos ici, ou cliquez pour choisir (PNG, JPEG, WebP, GIF, MP4, WebM)");
  root.append(h("h2", {}, "Bibliothèque de médias"), input, drop,
    state.media.length ? h("div", { class: "grid" }, state.media.map((m) =>
      h("div", { class: "card" }, thumb(m),
        h("div", { class: "name", title: m.original_name }, m.original_name),
        h("div", { class: "muted small" }, `${m.kind === "image" ? "Image" : "Vidéo"} · ${fmtSize(m.size)}`),
        h("button", { class: "danger small", onclick: act(async () => {
          if (!confirm(`Supprimer « ${m.original_name} » ?`)) return;
          await api(`/api/media/${m.id}`, { method: "DELETE" });
          await loadMedia(); render();
        }) }, "Supprimer"))))
      : h("p", { class: "muted" }, "Aucun média pour l'instant."));
}

// ---------- playlists ----------
const selectPlaylist = act(async (id) => {
  if (state.dirty && !confirm("Abandonner les modifications non enregistrées ?")) return;
  state.selectedPid = id; state.dirty = false; state.previewSrc = null;
  state.draft = currentPlaylist().items.map(toDraft);
  await Promise.all([loadMedia(), loadScreens()]);
  render();
});

function renderPlaylists(root) {
  const nameInput = h("input", { placeholder: "Nouvelle playlist…", maxlength: 120 });
  const create = act(async () => {
    const name = nameInput.value.trim();
    if (!name) return;
    const p = await api("/api/playlists", { method: "POST", body: { name } });
    await loadPlaylists(); await loadVersions();
    await selectPlaylist(p.id);
  });
  nameInput.addEventListener("keydown", (e) => { if (e.key === "Enter") create(); });
  const side = h("div", { class: "side" }, h("h2", {}, "Playlists"),
    h("div", { class: "row" }, nameInput, h("button", { class: "primary", onclick: create }, "+")),
    state.playlists.map((p) => h("button", { class: "pl" + (p.id === state.selectedPid ? " active" : ""),
      onclick: () => selectPlaylist(p.id) }, p.name)));
  const p = currentPlaylist();
  root.append(h("div", { class: "two-col" }, side,
    h("div", {}, p ? editor(p) : h("p", { class: "muted" }, "Sélectionnez ou créez une playlist."))));
}

function markDirty() {
  state.dirty = true;
  const f = $("#dirtyflag");
  if (f) f.hidden = false;
}

async function saveDraft() {
  const pid = state.selectedPid;
  const items = state.draft.map((it) => ({
    type: it.type,
    text: it.type === "text" ? it.text : null,
    media_id: it.type === "text" ? null : it.media_id,
    duration_s: it.duration_s,
  }));
  const saved = await api(`/api/playlists/${pid}/items`, { method: "PUT", body: items });
  const i = state.playlists.findIndex((p) => p.id === pid);
  state.playlists[i] = saved;
  state.draft = saved.items.map(toDraft);
  state.dirty = false;
}

function itemRow(it, i) {
  const move = (d) => {
    const j = i + d;
    if (j < 0 || j >= state.draft.length) return;
    [state.draft[i], state.draft[j]] = [state.draft[j], state.draft[i]];
    markDirty(); render();
  };
  let visual, body;
  if (it.type === "text") {
    visual = h("div", { class: "thumb" }, "T");
    body = h("textarea", { maxlength: 2000, placeholder: "Texte à afficher", value: it.text || "",
      oninput: (e) => { it.text = e.target.value; markDirty(); } });
  } else {
    const m = mediaById(it.media_id);
    visual = m ? thumb(m) : h("div", { class: "thumb" }, "?");
    body = h("div", {}, h("div", {}, m ? m.original_name : `Média #${it.media_id} introuvable`),
      h("div", { class: "muted small" }, it.type === "image" ? "Image" : "Vidéo (durée vide = lecture complète)"));
  }
  const dur = h("label", { class: "dur muted small" },
    h("input", { type: "number", min: 1, max: 3600, placeholder: it.type === "video" ? "auto" : "10",
      value: it.duration_s ?? "",
      oninput: (e) => { it.duration_s = e.target.value === "" ? null : parseInt(e.target.value, 10); markDirty(); } }), " s");
  return h("div", { class: "item" }, h("div", { class: "muted" }, i + 1), visual, body, dur,
    h("div", { class: "row" },
      h("button", { class: "small", title: "Monter", onclick: () => move(-1) }, "↑"),
      h("button", { class: "small", title: "Descendre", onclick: () => move(1) }, "↓"),
      h("button", { class: "small danger", title: "Retirer", onclick: () => { state.draft.splice(i, 1); markDirty(); render(); } }, "✕")));
}

function openPicker() {
  const body = state.media.length
    ? h("div", { class: "grid" }, state.media.map((m) => h("div", { class: "card pick", onclick: () => {
        state.draft.push({ type: m.kind, text: null, media_id: m.id, duration_s: m.kind === "image" ? 10 : null });
        markDirty(); render(); toast("Ajouté : " + m.original_name);
      } }, thumb(m), h("div", { class: "name" }, m.original_name))))
    : h("p", { class: "muted" }, "Aucun média : téléversez-en d'abord dans l'onglet Médias.");
  openModal("Ajouter un média (cliquez pour ajouter, plusieurs fois possible)", body);
}

function editor(p) {
  const nameInput = h("input", { value: p.name, maxlength: 120 });
  const preview = (src) => { state.previewSrc = src; render(); };
  const versions = [...(state.versions[p.id] || [])].reverse();
  const screenOptions = () => state.screens.map((s) => h("option", { value: s.id }, s.name));

  return h("div", {},
    h("div", { class: "panel" },
      h("div", { class: "row" }, nameInput,
        h("button", { onclick: act(async () => {
          await api(`/api/playlists/${p.id}`, { method: "PUT", body: { name: nameInput.value } });
          await loadPlaylists(); await loadVersions(); render(); toast("Renommée");
        }) }, "Renommer"),
        h("span", { class: "spacer" }),
        h("button", { class: "danger", onclick: act(async () => {
          if (!confirm(`Supprimer la playlist « ${p.name} » et ses versions ?`)) return;
          await api(`/api/playlists/${p.id}`, { method: "DELETE" });
          state.selectedPid = null; state.dirty = false;
          await loadPlaylists(); await loadVersions(); render();
        }) }, "Supprimer"))),

    h("div", { class: "panel" },
      h("div", { class: "row" }, h("h2", { style: "margin:0" }, "Contenu"), h("span", { class: "spacer" }),
        h("span", { id: "dirtyflag", class: "badge warn", hidden: !state.dirty }, "Modifications non enregistrées")),
      state.draft.length ? state.draft.map(itemRow) : h("p", { class: "muted" }, "Playlist vide. Ajoutez du texte ou des médias."),
      h("div", { class: "row", style: "margin-top:12px" },
        h("button", { onclick: () => { state.draft.push({ type: "text", text: "", media_id: null, duration_s: 10 }); markDirty(); render(); } }, "+ Texte"),
        h("button", { onclick: act(async () => { await loadMedia(); openPicker(); }) }, "+ Image / vidéo"),
        h("span", { class: "spacer" }),
        h("button", { onclick: act(async () => { await saveDraft(); render(); toast("Enregistré"); }) }, "Enregistrer"),
        h("button", { onclick: act(async () => { await saveDraft(); preview(`/player/?t=${Date.now()}#playlist=${p.id}`); }) }, "Enregistrer et prévisualiser"),
        h("button", { class: "primary", onclick: act(async () => {
          await saveDraft();
          const v = await api(`/api/playlists/${p.id}/publish`, { method: "POST" });
          await loadVersions(); render(); toast(`Version v${v.number} publiée`);
        }) }, "Publier une version"))),

    state.previewSrc ? h("div", { class: "panel" }, h("h2", {}, "Prévisualisation"),
      h("iframe", { class: "preview", src: state.previewSrc, allow: "autoplay; fullscreen" })) : null,

    h("div", { class: "panel" }, h("h2", {}, "Versions publiées"),
      versions.length ? versions.map((v) => {
        const sel = h("select", {}, screenOptions());
        const using = state.screens.filter((s) => s.version_id === v.id).map((s) => s.name);
        return h("div", { class: "vers" },
          h("strong", {}, `v${v.number}`), h("span", { class: "muted small" }, fmtDate(v.created_at)),
          using.length ? h("span", { class: "badge ok" }, "Diffusée sur : " + using.join(", ")) : null,
          h("span", { class: "spacer" }),
          h("button", { class: "small", onclick: () => preview(`/player/?t=${Date.now()}#version=${v.id}`) }, "Prévisualiser"),
          state.screens.length ? [sel, h("button", { class: "small primary", onclick: act(async () => {
            await api(`/api/screens/${sel.value}/assign`, { method: "POST", body: { version_id: v.id } });
            await loadScreens(); render(); toast(`v${v.number} diffusée`);
          }) }, "Diffuser")] : h("span", { class: "muted small" }, "Créez un écran pour diffuser"));
      }) : h("p", { class: "muted" }, "Aucune version publiée. Pour revenir en arrière, rediffusez une ancienne version.")));
}

// ---------- écrans ----------
function showToken(screen, token) {
  const url = `${location.origin}/player/#token=${token}`;
  const t = h("input", { class: "token", readonly: true, value: token });
  const u = h("input", { class: "token", readonly: true, value: url });
  openModal(`Jeton de l'écran « ${screen.name} »`,
    h("p", { class: "err" }, "Ce jeton n'est affiché qu'une seule fois. Copiez-le maintenant."),
    h("div", {}, "Jeton"), h("div", { class: "row" }, t, h("button", { onclick: () => copy(token) }, "Copier")),
    h("div", { style: "margin-top:10px" }, "Adresse à ouvrir sur l'écran (Chromium en mode kiosque)"),
    h("div", { class: "row" }, u, h("button", { onclick: () => copy(url) }, "Copier")));
}
function setStatus(el, s) {
  el.className = "badge " + (s.online ? "ok" : "off");
  el.textContent = s.online ? "● connecté" : "○ déconnecté";
}
function renderScreens(root) {
  const nameInput = h("input", { placeholder: "Nom de l'écran (ex. Accueil)", maxlength: 120 });
  const create = act(async () => {
    const name = nameInput.value.trim();
    if (!name) return;
    const s = await api("/api/screens", { method: "POST", body: { name } });
    await loadScreens(); render(); showToken(s, s.token);
  });
  nameInput.addEventListener("keydown", (e) => { if (e.key === "Enter") create(); });
  const versions = allVersions();

  root.append(h("h2", {}, "Écrans"),
    h("div", { class: "row", style: "margin-bottom:16px" }, nameInput, h("button", { class: "primary", onclick: create }, "Créer l'écran")),
    ...(state.screens.length ? state.screens.map((s) => {
      const status = h("span", { "data-status": s.id }); setStatus(status, s);
      const sel = h("select", { value: s.version_id ?? "" },
        h("option", { value: "" }, "— aucune —"),
        versions.map((v) => h("option", { value: v.id }, `${v.pname} · v${v.number}`)));
      return h("div", { class: "panel" },
        h("div", { class: "row" }, h("strong", {}, s.name), status,
          !s.has_token ? h("span", { class: "badge warn" }, "jeton révoqué") : null,
          h("span", { class: "spacer" }),
          h("span", { class: "muted small" }, "Dernière connexion : " + fmtDate(s.last_seen))),
        h("div", { class: "row", style: "margin-top:10px" },
          h("span", { class: "muted" }, "Diffuse :"), sel,
          h("button", { class: "small primary", onclick: act(async () => {
            await api(`/api/screens/${s.id}/assign`, { method: "POST", body: { version_id: sel.value === "" ? null : Number(sel.value) } });
            await loadScreens(); render(); toast("Affectation enregistrée");
          }) }, "Affecter"),
          h("span", { class: "spacer" }),
          h("button", { class: "small", onclick: () => openModal(`Aperçu : ${s.name}`,
            h("iframe", { class: "preview", src: `/player/?t=${Date.now()}#screen=${s.id}`, allow: "autoplay; fullscreen" })) }, "Voir l'écran"),
          h("button", { class: "small", onclick: act(async () => {
            if (!confirm("Générer un nouveau jeton ? L'ancien cessera de fonctionner immédiatement.")) return;
            const r = await api(`/api/screens/${s.id}/rotate-token`, { method: "POST" });
            await loadScreens(); render(); showToken(r, r.token);
          }) }, "Nouveau jeton"),
          s.has_token ? h("button", { class: "small danger", onclick: act(async () => {
            if (!confirm("Révoquer le jeton ? L'écran ne pourra plus se connecter.")) return;
            await api(`/api/screens/${s.id}/revoke-token`, { method: "POST" });
            await loadScreens(); render(); toast("Jeton révoqué");
          }) }, "Révoquer") : null));
    }) : [h("p", { class: "muted" }, "Aucun écran. Créez-en un ci-dessus.")]));
}

// Rafraîchit l'état connecté/déconnecté sans redessiner (conserve les sélections en cours).
setInterval(act(async () => {
  if (!state.token || state.view !== "screens") return;
  await loadScreens();
  for (const s of state.screens) { const el = document.querySelector(`[data-status="${s.id}"]`); if (el) setStatus(el, s); }
}), 10000);

if (state.token) boot(); else renderLogin();
})();
