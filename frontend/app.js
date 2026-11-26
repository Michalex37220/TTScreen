// Interface d'administration (sans étape de compilation). Aucune donnée n'est injectée via innerHTML.
(() => {
"use strict";

const state = {
  token: sessionStorage.getItem("ttscreen_admin") || "",
  me: null,  // identité renvoyée par /api/auth/me (username, role, legacy) : sert à afficher l'onglet Comptes
  view: "dashboard",
  playlists: [], media: [], screens: [], groups: [], schedules: [], stats: null, versions: {}, users: [], audit: [],
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
const loadGroups = async () => { state.groups = await api("/api/groups"); };
const loadSchedules = async () => { state.schedules = await api("/api/schedules"); };
const loadStats = async () => { state.stats = await api("/api/stats"); };
const loadVersions = async () => {
  const entries = await Promise.all(state.playlists.map(async (p) => [p.id, await api(`/api/playlists/${p.id}/versions`)]));
  state.versions = Object.fromEntries(entries);
};
const currentPlaylist = () => state.playlists.find((p) => p.id === state.selectedPid);
const allVersions = () => state.playlists.flatMap((p) => (state.versions[p.id] || []).map((v) => ({ ...v, pname: p.name })));
const versionLabel = (id) => { const v = allVersions().find((x) => x.id === id); return v ? `${v.pname} · v${v.number}` : "—"; };

// ---------- connexion ----------
// Deux modes acceptés par POST /api/auth/login :
// - identifiant + mot de passe d'un compte nommé (créé dans l'onglet Comptes) ;
// - si le champ identifiant est vide, le champ mot de passe est traité comme le jeton
//   admin historique (TTSCREEN_ADMIN_TOKEN), pour compatibilité avec l'ancien usage.
function logout() {
  // Déconnexion aussi côté serveur : révoque la session (le jeton cesse de fonctionner)
  if (state.token) fetch("/api/auth/logout", { method: "POST", headers: { Authorization: "Bearer " + state.token } }).catch(() => {});
  sessionStorage.removeItem("ttscreen_admin");
  state.token = "";
  state.me = null;
  renderLogin();
}
function renderLogin(message = "") {
  const user = h("input", { placeholder: "Identifiant (vide = jeton admin)", autofocus: true });
  const pass = h("input", { type: "password", placeholder: "Mot de passe ou jeton admin" });
  const err = h("div", { class: "err" }, message);
  const go = async () => {
    const body = user.value.trim()
      ? { username: user.value.trim(), password: pass.value }  // compte nommé
      : { token: pass.value };                                  // jeton admin historique
    const r = await fetch("/api/auth/login", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
    if (!r.ok) { err.textContent = "Identifiants invalides"; return; }
    state.token = (await r.json()).token;  // jeton de session (ou le jeton admin en mode historique)
    sessionStorage.setItem("ttscreen_admin", state.token);  // lu aussi par le lecteur de prévisualisation
    boot();
  };
  for (const input of [user, pass]) input.addEventListener("keydown", (e) => { if (e.key === "Enter") go(); });
  app.replaceChildren(h("div", { class: "login" },
    h("h1", {}, "TTscreen"),
    h("div", { class: "muted" }, "Compte utilisateur, ou jeton admin (TTSCREEN_ADMIN_TOKEN) si aucun compte n'existe"),
    user, pass, h("button", { class: "primary", onclick: go }, "Se connecter"), err));
}

// ---------- coque ----------
const boot = act(async () => {
  await Promise.all([loadPlaylists(), loadMedia(), loadScreens(), loadGroups(), loadSchedules(), loadStats()]);
  await loadVersions();
  state.me = await api("/api/auth/me");  // rôle de l'utilisateur connecté (admin voit l'onglet Comptes)
  // Onglet "Comptes" réservé au rôle admin : la gestion des utilisateurs et le journal
  // des opérations sensibles sont des routes /api protégées par require_admin côté serveur
  const tabs = [["dashboard", "Tableau de bord"], ["playlists", "Playlists"], ["media", "Médias"], ["screens", "Écrans"], ["groups", "Groupes"]];
  if (state.me.role === "admin") tabs.push(["accounts", "Comptes"]);
  app.replaceChildren(
    h("header", {}, h("strong", {}, "TTscreen"),
      h("nav", {}, tabs.map(([k, label]) => h("button", { "data-view": k, onclick: () => setView(k) }, label))),
      h("span", { class: "spacer" }),
      h("span", { class: "muted small" }, `${state.me.username} (${state.me.role})`),
      // le jeton admin historique n'a pas de mot de passe à changer
      state.me.legacy ? null : h("button", { class: "ghost", onclick: openPassword }, "Mon mot de passe"),
      h("button", { class: "ghost", onclick: logout }, "Déconnexion")),
    h("main", { id: "main" }));
  render();
});

const setView = act(async (v) => {
  if (state.dirty && !confirm("Abandonner les modifications non enregistrées ?")) return;
  state.dirty = false;
  if (v === "dashboard") await Promise.all([loadStats(), loadScreens(), loadGroups()]);
  if (v === "screens" || v === "groups") await Promise.all([loadScreens(), loadGroups(), loadSchedules(), loadVersions()]);
  if (v === "media") await loadMedia();
  if (v === "accounts") await Promise.all([loadUsers(), loadAudit()]);  // onglet Comptes : utilisateurs + journal
  state.view = v;
  render();
});

function render() {
  document.querySelectorAll("nav button").forEach((b) => b.classList.toggle("active", b.dataset.view === state.view));
  const main = $("#main");
  main.replaceChildren();
  ({ dashboard: renderDashboard, playlists: renderPlaylists, media: renderMedia, screens: renderScreens,
     groups: renderGroups, accounts: renderAccounts })[state.view](main);
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
  // Cibles de diffusion : groupes d'écrans (valeur "g:ID") et écrans individuels (valeur "s:ID")
  const targetOptions = () => [
    state.groups.length ? h("optgroup", { label: "Groupes d'écrans" }, state.groups.map((g) =>
      h("option", { value: "g:" + g.id }, `${g.name} (${g.screen_count} écran${g.screen_count > 1 ? "s" : ""})`))) : null,
    state.screens.length ? h("optgroup", { label: "Écrans" }, state.screens.map((s) => h("option", { value: "s:" + s.id }, s.name))) : null,
  ];

  return h("div", {},
    h("div", { class: "panel" },
      h("div", { class: "row" }, nameInput,
        h("button", { onclick: act(async () => {
          await api(`/api/playlists/${p.id}`, { method: "PUT", body: { name: nameInput.value } });
          await loadPlaylists(); await loadVersions(); render(); toast("Renommée");
        }) }, "Renommer"),
        h("span", { class: "spacer" }),
        h("button", { onclick: act(async () => {
          const c = await api(`/api/playlists/${p.id}/duplicate`, { method: "POST" });
          await loadPlaylists(); await loadVersions(); await selectPlaylist(c.id); toast("Playlist dupliquée");
        }) }, "Dupliquer"),
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
        const sel = h("select", {}, targetOptions());
        const using = [
          ...state.groups.filter((g) => g.version_id === v.id).map((g) => "groupe " + g.name),
          ...state.screens.filter((s) => s.version_id === v.id).map((s) => s.name),
        ];
        return h("div", { class: "vers" },
          h("strong", {}, `v${v.number}`), h("span", { class: "muted small" }, fmtDate(v.created_at)),
          using.length ? h("span", { class: "badge ok" }, "Diffusée sur : " + using.join(", ")) : null,
          h("span", { class: "spacer" }),
          h("button", { class: "small", onclick: () => preview(`/player/?t=${Date.now()}#version=${v.id}`) }, "Prévisualiser"),
          state.screens.length || state.groups.length ? [sel, h("button", { class: "small primary", onclick: act(async () => {
            const [kind, id] = sel.value.split(":");
            await api(kind === "g" ? `/api/groups/${id}/assign` : `/api/screens/${id}/assign`, { method: "POST", body: { version_id: v.id } });
            await Promise.all([loadScreens(), loadGroups()]); render(); toast(`v${v.number} diffusée`);
          }) }, "Diffuser")] : h("span", { class: "muted small" }, "Créez un écran ou un groupe pour diffuser"));
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
  const groupSel = h("select", {}, h("option", { value: "" }, "— sans groupe —"), state.groups.map((g) => h("option", { value: g.id }, g.name)));
  const create = act(async () => {
    const name = nameInput.value.trim();
    if (!name) return;
    const body = { name };
    if (groupSel.value) body.group_id = Number(groupSel.value);
    const s = await api("/api/screens", { method: "POST", body });
    await Promise.all([loadScreens(), loadGroups()]); render(); showToken(s, s.token);
  });
  nameInput.addEventListener("keydown", (e) => { if (e.key === "Enter") create(); });

  root.append(h("h2", {}, "Écrans"),
    h("div", { class: "row", style: "margin-bottom:16px" }, nameInput, groupSel, h("button", { class: "primary", onclick: create }, "Créer l'écran")),
    ...(state.screens.length ? state.screens.map(screenPanel) : [h("p", { class: "muted" }, "Aucun écran. Créez-en un ci-dessus.")]));
}

function screenPanel(s) {
  const status = h("span", { "data-status": s.id }); setStatus(status, s);
  const sel = h("select", { value: s.version_id ?? "" }, h("option", { value: "" }, "— aucun —"), versionOptions());
  const grp = h("select", { value: s.group_id ?? "" }, h("option", { value: "" }, "— sans groupe —"), state.groups.map((g) => h("option", { value: g.id }, g.name)));
  const rules = state.schedules.filter((r) => r.screen_id === s.id).length;
  return h("div", { class: "panel" },
    h("div", { class: "row" }, h("strong", {}, s.name), status,
      s.group_name ? h("span", { class: "badge ok" }, "groupe : " + s.group_name) : null,
      !s.has_token ? h("span", { class: "badge warn" }, "jeton révoqué") : null,
      h("span", { class: "spacer" }),
      h("span", { class: "muted small" }, "Dernière connexion : " + fmtDate(s.last_seen))),
    h("div", { class: "row", style: "margin-top:8px" }, h("span", { class: "muted" }, "Affiche actuellement :"), effectiveText(s)),
    h("div", { class: "row", style: "margin-top:10px" },
      h("span", { class: "muted" }, "Contenu propre :"), sel,
      h("button", { class: "small primary", onclick: act(async () => {
        await api(`/api/screens/${s.id}/assign`, { method: "POST", body: { version_id: sel.value === "" ? null : Number(sel.value) } });
        await refreshScreensAndGroups(); toast("Affectation enregistrée");
      }) }, "Affecter"),
      h("span", { class: "muted", style: "margin-left:12px" }, "Groupe :"), grp,
      h("button", { class: "small", onclick: act(async () => {
        await api(`/api/screens/${s.id}/group`, { method: "POST", body: { group_id: grp.value === "" ? null : Number(grp.value) } });
        await refreshScreensAndGroups(); toast("Groupe enregistré");
      }) }, "Déplacer")),
    h("div", { class: "row", style: "margin-top:10px" },
      h("button", { class: "small", onclick: () => renameModal(`Renommer l'écran « ${s.name} »`, s.name, async (name) => {
        await api(`/api/screens/${s.id}`, { method: "PUT", body: { name } }); await refreshScreensAndGroups(); toast("Écran renommé");
      }) }, "Renommer"),
      h("button", { class: "small", onclick: () => openSchedule("screen", s) }, `Planning (${rules})`),
      h("span", { class: "spacer" }),
      h("button", { class: "small", onclick: () => openModal(`Aperçu : ${s.name}`,
        h("iframe", { class: "preview", src: `/player/?t=${Date.now()}#screen=${s.id}`, allow: "autoplay; fullscreen" })) }, "Voir l'écran"),
      h("button", { class: "small", onclick: act(async () => {
        if (!confirm("Générer un nouveau jeton ? L'ancien cessera de fonctionner immédiatement.")) return;
        const r = await api(`/api/screens/${s.id}/rotate-token`, { method: "POST" });
        await refreshScreensAndGroups(); showToken(r, r.token);
      }) }, "Nouveau jeton"),
      s.has_token ? h("button", { class: "small danger", onclick: act(async () => {
        if (!confirm("Révoquer le jeton ? L'écran ne pourra plus se connecter.")) return;
        await api(`/api/screens/${s.id}/revoke-token`, { method: "POST" });
        await refreshScreensAndGroups(); toast("Jeton révoqué");
      }) }, "Révoquer") : null,
      h("button", { class: "small danger", onclick: act(async () => {
        if (!confirm(`Supprimer l'écran « ${s.name} » ? Son jeton cessera de fonctionner et son planning sera supprimé.`)) return;
        await api(`/api/screens/${s.id}`, { method: "DELETE" });
        await refreshScreensAndGroups(); toast("Écran supprimé");
      }) }, "Supprimer")));
}

// Rafraîchit l'état connecté/déconnecté sans redessiner (conserve les sélections en cours).
setInterval(act(async () => {
  if (!state.token || state.view !== "screens") return;
  await loadScreens();
  for (const s of state.screens) { const el = document.querySelector(`[data-status="${s.id}"]`); if (el) setStatus(el, s); }
}), 10000);

// ---------- libellés communs ----------
const SOURCE_LABEL = {
  screen_schedule: "planning de l'écran", screen: "contenu propre à l'écran",
  group_schedule: "planning du groupe", group: "contenu du groupe",
};
const DAY_NAMES = ["Lun", "Mar", "Mer", "Jeu", "Ven", "Sam", "Dim"];
const plural = (n, one, many) => `${n} ${n > 1 ? many : one}`;
const noVersion = (v) => v === null || v === undefined;

function effectiveText(s) {
  if (noVersion(s.effective_version_id)) return h("span", { class: "muted" }, "aucun contenu");
  const via = SOURCE_LABEL[s.effective_source];
  return h("span", {}, h("strong", {}, versionLabel(s.effective_version_id)), via ? h("span", { class: "muted small" }, ` (${via})`) : null);
}
const versionOptions = () => allVersions().map((v) => h("option", { value: v.id }, `${v.pname} · v${v.number}`));

function renameModal(title, current, onSave) {
  const input = h("input", { value: current, maxlength: 120 });
  const close = openModal(title, input,
    h("button", { class: "primary", onclick: act(async () => {
      const v = input.value.trim();
      if (!v) return;
      await onSave(v);
      close();
    }) }, "Enregistrer"));
  input.focus();
}

// ---------- mot de passe personnel ----------
function openPassword() {
  const cur = h("input", { type: "password", placeholder: "Mot de passe actuel", maxlength: 255 });
  const n1 = h("input", { type: "password", placeholder: "Nouveau mot de passe (8 caractères min.)", maxlength: 255 });
  const n2 = h("input", { type: "password", placeholder: "Confirmer le nouveau mot de passe", maxlength: 255 });
  const close = openModal("Changer mon mot de passe", cur, n1, n2,
    h("p", { class: "muted small" }, "Vos autres sessions seront déconnectées."),
    h("button", { class: "primary", onclick: act(async () => {
      if (n1.value.length < 8) { toast("Nouveau mot de passe : 8 caractères minimum", true); return; }
      if (n1.value !== n2.value) { toast("La confirmation ne correspond pas", true); return; }
      await api("/api/auth/password", { method: "POST", body: { current_password: cur.value, new_password: n1.value } });
      close(); toast("Mot de passe changé");
    }) }, "Valider"));
  cur.focus();
}

// ---------- planning (écran ou groupe) ----------
function describeRule(r) {
  const days = r.days.length === 7 ? "tous les jours" : r.days.map((d) => DAY_NAMES[d]).join(" ");
  return `${days} · ${r.start}–${r.end}`;
}
// kind : "screen" | "group" ; target : l'écran ou le groupe
function openSchedule(kind, target) {
  const body = h("div", {});
  const redraw = () => {
    const rules = state.schedules.filter((r) => r[kind + "_id"] === target.id);
    const ver = h("select", {}, versionOptions());
    const dayBoxes = DAY_NAMES.map((name, i) => h("label", { class: "day" }, h("input", { type: "checkbox", "data-day": i, checked: i < 5 }), name));
    const start = h("input", { type: "time", value: "08:00" });
    const end = h("input", { type: "time", value: "18:00" });
    const midnight = h("input", { type: "checkbox", onchange: () => { end.disabled = midnight.checked; } });
    const add = act(async () => {
      const days = dayBoxes.map((l) => l.querySelector("input")).filter((c) => c.checked).map((c) => Number(c.dataset.day));
      if (!ver.value) { toast("Publiez d'abord une version dans l'onglet Playlists", true); return; }
      if (!days.length) { toast("Choisissez au moins un jour", true); return; }
      await api("/api/schedules", { method: "POST", body: {
        [kind + "_id"]: target.id, version_id: Number(ver.value), days, start: start.value, end: midnight.checked ? "24:00" : end.value } });
      await Promise.all([loadSchedules(), loadScreens(), loadGroups()]);
      redraw(); render(); toast("Règle ajoutée");
    });
    // replaceChildren (DOM natif) n'aplatit pas les tableaux, contrairement à h() : on le fait ici
    body.replaceChildren(...[
      h("p", { class: "muted small" }, "Hors des plages ci-dessous, l'écran revient à son contenu habituel. Si plusieurs règles se chevauchent, la plus récente l'emporte. Heures du fuseau du serveur."),
      rules.length ? rules.map((r) => h("div", { class: "vers" },
        h("strong", {}, describeRule(r)), h("span", { class: "muted small" }, "→ " + versionLabel(r.version_id)), h("span", { class: "spacer" }),
        h("button", { class: "small danger", onclick: act(async () => {
          await api(`/api/schedules/${r.id}`, { method: "DELETE" });
          await Promise.all([loadSchedules(), loadScreens(), loadGroups()]);
          redraw(); render(); toast("Règle supprimée");
        }) }, "Supprimer")))
        : h("p", { class: "muted" }, "Aucune règle : l'écran affiche en permanence son contenu habituel."),
      h("h3", {}, "Ajouter une règle"),
      h("div", { class: "row" }, h("span", { class: "muted" }, "Diffuser"), ver),
      h("div", { class: "row days" }, dayBoxes),
      h("div", { class: "row" }, h("span", { class: "muted" }, "de"), start, h("span", { class: "muted" }, "à"), end,
        h("label", { class: "day" }, midnight, "jusqu'à minuit")),
      h("button", { class: "primary", onclick: add }, "Ajouter la règle"),
    ].flat());
  };
  redraw();
  openModal(`Planning : ${target.name}`, body);
}

// ---------- tableau de bord ----------
function stat(label, value, cls = "") {
  return h("div", { class: "stat " + cls }, h("div", { class: "stat-value" }, String(value)), h("div", { class: "muted small" }, label));
}
function renderDashboard(root) {
  const st = state.stats;
  if (!st) { root.append(h("p", { class: "muted" }, "Chargement…")); return; }
  const watch = state.screens.filter((s) => !s.online || noVersion(s.effective_version_id));
  const allOnline = st.screens.total > 0 && st.screens.online === st.screens.total;
  root.append(h("h2", {}, "Tableau de bord"),
    h("div", { class: "stats" },
      stat("écrans connectés", `${st.screens.online} / ${st.screens.total}`, st.screens.total === 0 ? "" : allOnline ? "ok" : "warn"),
      stat("écrans sans contenu", st.screens.without_content, st.screens.without_content ? "warn" : ""),
      stat("groupes d'écrans", st.groups),
      stat("playlists", st.playlists),
      stat("médias", `${st.media.count} · ${st.media.bytes ? fmtSize(st.media.bytes) : "0 Ko"}`),
      stat("règles de planning", st.schedules)),
    h("div", { class: "panel" }, h("h2", {}, "Écrans à surveiller"),
      watch.length ? watch.map((s) => h("div", { class: "vers" },
        h("strong", {}, s.name),
        !s.online ? h("span", { class: "badge off" }, "○ déconnecté") : null,
        noVersion(s.effective_version_id) ? h("span", { class: "badge warn" }, "sans contenu") : null,
        s.group_name ? h("span", { class: "muted small" }, "groupe " + s.group_name) : null,
        h("span", { class: "spacer" }),
        h("span", { class: "muted small" }, "Dernière connexion : " + fmtDate(s.last_seen))))
        : h("p", { class: "muted" }, state.screens.length ? "Tout va bien : tous les écrans sont connectés et affichent du contenu."
          : "Aucun écran pour l'instant : créez-en un dans l'onglet Écrans.")));
}

// ---------- groupes d'écrans ----------
function renderGroups(root) {
  const nameInput = h("input", { placeholder: "Nom du groupe (ex. Hall d'entrée)", maxlength: 120 });
  const create = act(async () => {
    const name = nameInput.value.trim();
    if (!name) return;
    await api("/api/groups", { method: "POST", body: { name } });
    await loadGroups(); render(); toast("Groupe créé");
  });
  nameInput.addEventListener("keydown", (e) => { if (e.key === "Enter") create(); });
  root.append(h("h2", {}, "Groupes d'écrans"),
    h("p", { class: "muted small" }, "Tous les écrans d'un groupe affichent le même contenu. Un écran qui a son propre contenu fixé, ou un planning actif, le garde."),
    h("div", { class: "row", style: "margin-bottom:16px" }, nameInput, h("button", { class: "primary", onclick: create }, "Créer le groupe")),
    ...(state.groups.length ? state.groups.map(groupPanel) : [h("p", { class: "muted" }, "Aucun groupe. Créez-en un ci-dessus, puis ajoutez-y des écrans.")]));
}
const refreshScreensAndGroups = async () => { await Promise.all([loadScreens(), loadGroups(), loadSchedules()]); render(); };

function groupPanel(g) {
  const members = state.screens.filter((s) => s.group_id === g.id);
  const others = state.screens.filter((s) => s.group_id !== g.id);
  const sel = h("select", { value: g.version_id ?? "" }, h("option", { value: "" }, "— aucun —"), versionOptions());
  const add = h("select", {}, h("option", { value: "" }, "+ Ajouter un écran…"),
    others.map((s) => h("option", { value: s.id }, s.group_name ? `${s.name} (actuellement : ${s.group_name})` : s.name)));
  const rules = state.schedules.filter((r) => r.group_id === g.id).length;
  return h("div", { class: "panel" },
    h("div", { class: "row" }, h("strong", {}, g.name), h("span", { class: "badge ok" }, plural(g.screen_count, "écran", "écrans")),
      g.version_id ? h("span", { class: "muted small" }, "contenu : " + versionLabel(g.version_id)) : h("span", { class: "muted small" }, "aucun contenu"),
      h("span", { class: "spacer" }),
      h("button", { class: "small", onclick: () => renameModal(`Renommer « ${g.name} »`, g.name, async (name) => {
        await api(`/api/groups/${g.id}`, { method: "PUT", body: { name } }); await refreshScreensAndGroups(); toast("Groupe renommé");
      }) }, "Renommer"),
      h("button", { class: "small", onclick: () => openSchedule("group", g) }, `Planning (${rules})`),
      h("button", { class: "small danger", onclick: act(async () => {
        if (!confirm(`Supprimer le groupe « ${g.name} » ? Ses ${plural(g.screen_count, "écran", "écrans")} ne seront pas supprimés mais perdront le contenu du groupe.`)) return;
        await api(`/api/groups/${g.id}`, { method: "DELETE" }); await refreshScreensAndGroups(); toast("Groupe supprimé");
      }) }, "Supprimer")),
    h("div", { class: "row", style: "margin-top:10px" }, h("span", { class: "muted" }, "Diffuse sur tout le groupe :"), sel,
      h("button", { class: "small primary", onclick: act(async () => {
        await api(`/api/groups/${g.id}/assign`, { method: "POST", body: { version_id: sel.value === "" ? null : Number(sel.value) } });
        await refreshScreensAndGroups(); toast("Contenu du groupe enregistré");
      }) }, "Affecter")),
    h("div", { class: "row chips", style: "margin-top:10px" },
      members.length ? members.map((s) => h("span", { class: "chip" },
        h("span", { class: s.online ? "dot ok" : "dot off", title: s.online ? "connecté" : "déconnecté" }), s.name,
        s.effective_source && s.effective_source !== "group" ? h("span", { class: "badge warn", title: "Cet écran n'affiche pas le contenu du groupe" }, SOURCE_LABEL[s.effective_source]) : null,
        h("button", { class: "ghost small", title: "Retirer du groupe", onclick: act(async () => {
          await api(`/api/screens/${s.id}/group`, { method: "POST", body: { group_id: null } }); await refreshScreensAndGroups();
        }) }, "✕")))
        : h("span", { class: "muted" }, "Aucun écran dans ce groupe.")),
    others.length ? h("div", { class: "row", style: "margin-top:10px" }, add,
      h("button", { class: "small", onclick: act(async () => {
        if (!add.value) return;
        await api(`/api/screens/${add.value}/group`, { method: "POST", body: { group_id: g.id } }); await refreshScreensAndGroups();
      }) }, "Ajouter au groupe")) : null);
}

// ---------- comptes et journal (onglet réservé au rôle admin) ----------
const loadUsers = async () => { state.users = await api("/api/users"); };
const loadAudit = async () => { state.audit = await api("/api/audit?limit=200"); };  // 200 dernières opérations

function renderAccounts(root) {
  // Formulaire de création de compte : le mot de passe transite sur la connexion HTTPS
  // mais n'est jamais stocké en clair côté serveur (haché PBKDF2, voir backend/app/auth.py)
  const uName = h("input", { placeholder: "Identifiant", maxlength: 50 });
  const uPass = h("input", { type: "password", placeholder: "Mot de passe (8 caractères min.)", maxlength: 255 });
  const uRole = h("select", {}, ["viewer", "editor", "admin"].map((r) => h("option", { value: r }, r)));
  const createUser = act(async () => {
    if (!uName.value.trim() || uPass.value.length < 8) { toast("Identifiant requis, mot de passe de 8 caractères minimum", true); return; }
    await api("/api/users", { method: "POST", body: { username: uName.value.trim(), password: uPass.value, role: uRole.value } });
    uName.value = ""; uPass.value = "";
    await loadUsers(); render(); toast("Compte créé");
  });

  const usersPanel = h("div", { class: "panel" },
    h("h2", {}, "Comptes utilisateurs"),
    h("div", { class: "row" }, uName, uPass, uRole, h("button", { class: "primary", onclick: createUser }, "Créer le compte")),
    h("p", { class: "muted small" }, "viewer = lecture seule · editor = contenu, publication, écrans · admin = tout, y compris cette page"),
    state.users.length ? state.users.map((u) => h("div", { class: "vers" },
      h("strong", {}, u.username),
      h("span", { class: "badge " + (u.role === "admin" ? "warn" : "ok") }, u.role),
      !u.active ? h("span", { class: "badge off" }, "désactivé") : null,
      h("span", { class: "muted small" }, "Dernière connexion : " + fmtDate(u.last_login)),
      h("span", { class: "spacer" }),
      // Désactiver/activer révoque immédiatement les sessions du compte (reconnexion obligatoire)
      u.active
        ? h("button", { class: "small danger", onclick: act(async () => {
            if (!confirm(`Désactiver « ${u.username} » ? Ses sessions sont révoquées.`)) return;
            await api(`/api/users/${u.id}`, { method: "PATCH", body: { active: false } });
            await loadUsers(); render();
          }) }, "Désactiver")
        : h("button", { class: "small", onclick: act(async () => {
            await api(`/api/users/${u.id}`, { method: "PATCH", body: { active: true } });
            await loadUsers(); render();
          }) }, "Activer"),
      h("button", { class: "small", onclick: () => {
        const p = h("input", { type: "password", placeholder: "Nouveau mot de passe (8 min.)" });
        const close = openModal(`Mot de passe de « ${u.username} »`, p,
          h("button", { class: "primary", onclick: act(async () => {
            if (p.value.length < 8) { toast("8 caractères minimum", true); return; }
            await api(`/api/users/${u.id}`, { method: "PATCH", body: { password: p.value } });
            close(); toast("Mot de passe changé (sessions révoquées)");
          }) }, "Valider"));
      } }, "Mot de passe"),
      h("button", { class: "small danger", onclick: act(async () => {
        if (!confirm(`Supprimer définitivement « ${u.username} » ?`)) return;
        await api(`/api/users/${u.id}`, { method: "DELETE" });
        await loadUsers(); render();
      }) }, "Supprimer")
    )) : h("p", { class: "muted" }, "Aucun compte nommé. Le jeton admin historique (TTSCREEN_ADMIN_TOKEN) reste utilisable."));

  // Journal des opérations sensibles (table audit_log côté serveur) : qui a fait quoi, quand, depuis quelle IP
  const auditPanel = h("div", { class: "panel" },
    h("div", { class: "row" }, h("h2", { style: "margin:0" }, "Journal des opérations sensibles"),
      h("span", { class: "spacer" }),
      h("button", { class: "small", onclick: act(async () => { await loadAudit(); render(); }) }, "Rafraîchir")),
    state.audit.length ? state.audit.map((a) => h("div", { class: "vers" },
      h("span", { class: "muted small" }, fmtDate(a.created_at)),
      h("strong", {}, a.username),
      h("span", { class: "badge ok" }, a.action),
      a.target ? h("span", { class: "muted small" }, a.target) : null,
      a.detail ? h("span", { class: "muted small" }, a.detail) : null,
      a.ip ? h("span", { class: "muted small" }, a.ip) : null
    )) : h("p", { class: "muted" }, "Aucune opération journalisée pour l'instant."));

  root.append(h("h2", {}, "Comptes"), usersPanel, auditPanel);
}

if (state.token) boot(); else renderLogin();
})();
