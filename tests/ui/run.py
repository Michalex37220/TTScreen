"""Test de l'interface d'administration dans un vrai navigateur (Playwright), contre tests/ui/mock_server.py.
Usage : ./scripts/test-ui.sh   (pip install playwright && playwright install chromium)"""
import os, sys
from playwright.sync_api import sync_playwright, expect

BASE = os.environ.get("TTSCREEN_UI_BASE", "http://127.0.0.1:8765")
SHOTS = os.environ.get("TTSCREEN_UI_SHOTS", "/tmp")
errors, results = [], []
def check(label, cond, extra=""):
    results.append((label, bool(cond)))
    print(("OK   " if cond else "FAIL ") + label + (f"  [{extra}]" if extra and not cond else ""))

with sync_playwright() as p:
    b = p.chromium.launch(headless=True)
    page = b.new_page(viewport={"width": 1280, "height": 900})
    page.on("pageerror", lambda e: errors.append(f"pageerror: {e}"))
    # le 403 du test « mauvais mot de passe » est attendu : le navigateur journalise toute réponse 4xx
    page.on("console", lambda m: errors.append(f"console.{m.type}: {m.text}") if m.type == "error" and "403" not in m.text else None)
    page.on("dialog", lambda d: d.accept())   # confirm(...)
    page.goto(BASE + "/admin/")
    page.get_by_placeholder("Mot de passe ou jeton admin").fill("x")
    page.get_by_role("button", name="Se connecter").click()

    # --- tableau de bord (vue par défaut)
    expect(page.get_by_role("heading", name="Tableau de bord")).to_be_visible()
    body = page.locator("main, body").first.inner_text()
    check("dashboard: écrans connectés 1 / 2", "1 / 2" in body)
    check("dashboard: écran déconnecté listé", "Caisse" in body and "déconnecté" in body)
    check("dashboard: bouton Mon mot de passe", page.get_by_role("button", name="Mon mot de passe").count() == 1)
    page.screenshot(path=SHOTS + "/ttscreen_ui_1_dashboard.png")

    # --- groupes : création
    page.get_by_role("button", name="Groupes", exact=True).click()
    page.get_by_placeholder("Nom du groupe").fill("Hall d'entrée")
    page.get_by_role("button", name="Créer le groupe").click()
    expect(page.get_by_text("Hall d'entrée").first).to_be_visible()
    check("groupes: groupe créé (0 écran)", "0 écran" in page.locator("body").inner_text())

    # --- écrans : création directement dans le groupe
    page.get_by_role("button", name="Écrans", exact=True).click()
    page.get_by_placeholder("Nom de l'écran").fill("Vitrine")
    page.locator("select").first.select_option(label="Hall d'entrée")
    page.get_by_role("button", name="Créer l'écran").click()
    expect(page.get_by_text("groupe : Hall d'entrée")).to_be_visible()
    check("écrans: badge de groupe affiché", True)
    # fermer la fenêtre du jeton si présente
    for name in ["Fermer", "OK", "✕"]:
        btn = page.get_by_role("button", name=name, exact=True)
        if btn.count() and btn.first.is_visible():
            btn.first.click(); break
    page.keyboard.press("Escape")

    # --- diffusion sur le groupe
    page.get_by_role("button", name="Groupes", exact=True).click()
    panel = page.locator(".panel").filter(has_text="Hall d'entrée").first
    expect(panel).to_contain_text("1 écran")
    check("groupes: le groupe compte 1 écran", True)
    panel.locator("select").first.select_option(label="Promo · v1")
    panel.get_by_role("button", name="Affecter").click()
    expect(page.get_by_text("contenu : Promo · v1").first).to_be_visible()
    check("groupes: contenu du groupe affiché", True)

    # l'écran du groupe hérite du contenu
    page.get_by_role("button", name="Écrans", exact=True).click()
    vit = page.locator(".panel").filter(has_text="Vitrine").first
    expect(vit).to_contain_text("contenu du groupe")
    t = vit.inner_text()
    check("écrans: Vitrine affiche le contenu du groupe", "Promo" in t and "contenu du groupe" in t, t)
    page.screenshot(path=SHOTS + "/ttscreen_ui_2_screens.png", full_page=True)

    # --- planning d'un écran
    vit.get_by_role("button", name="Planning (0)").click()
    modal = page.locator(".modal")
    expect(modal.get_by_text("Aucune règle")).to_be_visible()
    modal.get_by_role("button", name="Ajouter la règle").click()
    expect(modal.get_by_text("Lun Mar Mer Jeu Ven · 08:00–18:00")).to_be_visible()
    check("planning: règle ajoutée et décrite", True)
    page.screenshot(path=SHOTS + "/ttscreen_ui_3_schedule_modal.png")
    modal.get_by_role("button", name="Supprimer").click()
    expect(modal.get_by_text("Aucune règle")).to_be_visible()
    check("planning: règle supprimée", True)
    modal.get_by_role("button", name="Ajouter la règle").click()   # on la remet : l'écran doit passer en « planning de l'écran »
    expect(modal.get_by_text("Lun Mar Mer Jeu Ven")).to_be_visible()
    page.keyboard.press("Escape")
    page.mouse.click(5, 5)
    page.wait_for_timeout(200)
    vit = page.locator(".panel").filter(has_text="Vitrine").first
    check("écrans: planning actif reflété", "planning de l'écran" in vit.inner_text(), vit.inner_text())

    # --- renommer un écran
    vit.get_by_role("button", name="Renommer").click()
    page.locator(".modal input").fill("Vitrine 2")
    page.locator(".modal").get_by_role("button", name="Enregistrer").click()
    expect(page.get_by_text("Vitrine 2").first).to_be_visible()
    check("écrans: renommage", True)

    # --- déplacer hors du groupe, puis supprimer un écran
    v2 = page.locator(".panel").filter(has_text="Vitrine 2").first
    v2.locator("select").nth(1).select_option(label="— sans groupe —")
    v2.get_by_role("button", name="Déplacer").click()
    page.wait_for_timeout(300)
    check("écrans: sorti du groupe", "groupe : Hall" not in page.locator(".panel").filter(has_text="Vitrine 2").first.inner_text())
    page.locator(".panel").filter(has_text="Vitrine 2").first.get_by_role("button", name="Supprimer").click()
    page.wait_for_timeout(300)
    check("écrans: suppression", "Vitrine 2" not in page.locator("body").inner_text())

    # --- playlists : dupliquer + diffuser vers un groupe
    page.get_by_role("button", name="Playlists", exact=True).click()
    page.get_by_text("Promo", exact=True).first.click()
    page.get_by_role("button", name="Dupliquer").click()
    expect(page.get_by_text("Copie de Promo").first).to_be_visible()
    check("playlists: duplication", True)
    page.get_by_text("Promo", exact=True).first.click()
    expect(page.locator('optgroup[label="Groupes d\'écrans"]').first).to_be_attached()
    check("playlists: diffusion propose les groupes", True)
    opt = page.locator('optgroup[label="Groupes d\'écrans"] option').first.inner_text()
    check("playlists: libellé du groupe avec son nombre d'écrans", "Hall d'entrée" in opt and "écran" in opt, opt)
    page.screenshot(path=SHOTS + "/ttscreen_ui_4_playlists.png", full_page=True)

    # --- changement de mot de passe
    page.get_by_role("button", name="Mon mot de passe").click()
    m = page.locator(".modal")
    m.get_by_placeholder("Mot de passe actuel").fill("mauvais")
    m.get_by_placeholder("Nouveau mot de passe (8 caractères min.)").fill("nouveau-secret-9")
    m.get_by_placeholder("Confirmer").fill("nouveau-secret-9")
    m.get_by_role("button", name="Valider").click()
    expect(page.get_by_text("Mot de passe actuel incorrect")).to_be_visible()
    check("mot de passe: erreur serveur affichée", True)
    m.get_by_placeholder("Mot de passe actuel").fill("ancien123")
    m.get_by_role("button", name="Valider").click()
    expect(page.get_by_text("Mot de passe changé")).to_be_visible()
    check("mot de passe: changement confirmé", True)

    # --- suppression du groupe
    page.get_by_role("button", name="Groupes", exact=True).click()
    page.locator(".panel").filter(has_text="Hall d'entrée").first.get_by_role("button", name="Supprimer").click()
    expect(page.get_by_text("Aucun groupe")).to_be_visible()
    check("groupes: suppression", True)
    b.close()

print()
print("Erreurs JS/console :", errors if errors else "aucune")
failed = [l for l, ok in results if not ok]
print(f"{len(results) - len(failed)}/{len(results)} vérifications OK")
sys.exit(1 if failed or errors else 0)
