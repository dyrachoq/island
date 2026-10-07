"""
UI-тесты острова и окна настроек в headless Chromium (тот же движок, что WebView2).
Rust-сторона подменена моком из mock_tauri.js.

Запуск:  pip install playwright && playwright install chromium
         python tests/test_ui.py
"""
import json, os, sys, time
from pathlib import Path
from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "src"
SHOTS = Path(os.environ.get("SHOTS", ROOT / "tests" / "shots"))
SHOTS.mkdir(parents=True, exist_ok=True)
MOCK = (ROOT / "tests" / "mock_tauri.js").read_text(encoding="utf-8")

results = []

def check(name, cond, detail=""):
    results.append((name, bool(cond), detail))
    print(("  OK   " if cond else "  FAIL ") + name + (f"  [{detail}]" if detail and not cond else ""))

def size(page):
    return page.evaluate("(() => { const r = document.querySelector('#island').getBoundingClientRect(); return [Math.round(r.width), Math.round(r.height)]; })()")

def cls(page):
    return page.evaluate("document.querySelector('#island').className")

def active_layer(page):
    return page.evaluate("[...document.querySelectorAll('.layer.on')].map(l => l.dataset.layer)")

def emit(page, ev, payload):
    page.evaluate("([e, p]) => window.__emit(e, p)", [ev, payload])

def calls(page, cmd):
    return page.evaluate("(c) => window.__callsOf(c)", cmd)

def settle(page, ms=900):
    page.wait_for_timeout(ms)

def shot(page, name):
    page.screenshot(path=str(SHOTS / f"{name}.png"))

DESKTOP_BG = "html { background: linear-gradient(135deg,#5b7fa6,#c58fb4) !important; }"

def open_island(browser, **overrides):
    page = browser.new_page(viewport={"width": 480, "height": 360})
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.on("console", lambda m: errors.append(m.text) if m.type == "error" and "ERR_UNKNOWN_URL_SCHEME" not in m.text else None)
    page.add_init_script(MOCK)
    if overrides:
        page.add_init_script("Object.assign(window.__responses, " + json.dumps(overrides) + ")")
    page.goto((SRC / "island.html").as_uri())
    page.add_style_tag(content=DESKTOP_BG)
    settle(page, 400)
    return page, errors

def hover(page, on=True):
    emit(page, "hover", on)

MEDIA = {
    "active": True, "title": "Нить", "artist": "Тестовый исполнитель", "album": "Альбом",
    "app": "Spotify", "source": "Spotify.exe", "playing": True, "canPrev": True, "canNext": True,
    "canPlayPause": True, "positionMs": 30000, "durationMs": 200000, "updatedAt": 0,
}

def art_png():
    import base64, io
    from PIL import Image, ImageDraw
    im = Image.new("RGB", (64, 64), (220, 60, 90))
    ImageDraw.Draw(im).ellipse([16, 16, 48, 48], fill=(250, 200, 60))
    buf = io.BytesIO(); im.save(buf, "PNG")
    return "data:image/png;base64," + base64.b64encode(buf.getvalue()).decode()

with sync_playwright() as p:
    browser = p.chromium.launch()

    # ---------------------------------------------------------------- базовое
    print("Остров: базовое поведение")
    page, errors = open_island(browser)
    check("стартует компактным 126×34", size(page) == [126, 34], size(page))
    rect = calls(page, "set_hit_rect")
    check("сообщает Rust область острова", len(rect) > 0 and rect[-1][1]["w"] > 120, rect[-1:] )
    shot(page, "01_compact")

    hover(page, True); settle(page)
    check("наведение раскрывает остров 400×196", size(page) == [400, 196], size(page))
    check("по умолчанию открыта вкладка музыки", page.evaluate("document.querySelector('.tab.active').dataset.tab") == "music")
    check("пустой плеер: «Ничего не играет»", page.inner_text("#m-title") == "Ничего не играет")
    check("кнопки плеера выключены без музыки", page.evaluate("document.querySelector('#m-play').disabled"))
    big = calls(page, "set_hit_rect")[-1][1]
    check("область клика выросла вместе с островом", big["w"] >= 400 and big["h"] >= 196, big)
    shot(page, "02_expanded_empty")

    hover(page, False); settle(page)
    check("уход мыши сворачивает остров", size(page) == [126, 34], size(page))

    # клик-режим
    emit(page, "settings-changed", {**page.evaluate("window.__responses.get_settings"), "expandOnHover": False})
    hover(page, True); settle(page, 500)
    check("в режиме «по клику» наведение не раскрывает", size(page) == [126, 34], size(page))
    page.click("#island"); settle(page)
    check("клик раскрывает", size(page) == [400, 196], size(page))
    page.mouse.move(470, 350)  # убираем настоящий курсор, иначе сработает CSS :hover
    hover(page, False); settle(page, 1700)
    check("и уход мыши потом сворачивает", size(page) == [126, 34], size(page))
    emit(page, "settings-changed", page.evaluate("window.__responses.get_settings"))

    page.click("#island", button="right")
    check("правый клик открывает настройки", len(calls(page, "open_settings")) == 1)

    emit(page, "fullscreen", True); settle(page)
    hidden_rect = calls(page, "set_hit_rect")[-1][1]
    check("в полноэкранном режиме остров прячется", "hidden" in cls(page))
    check("и перестаёт ловить мышь", hidden_rect["w"] == 0, hidden_rect)
    hover(page, True); settle(page, 400)
    check("спрятанный остров не раскрывается", "s-compact" in cls(page), cls(page))
    hover(page, False)
    emit(page, "fullscreen", False); settle(page)
    check("после выхода из полноэкранного возвращается", "hidden" not in cls(page))
    check("нет ошибок JS", not errors, errors)
    page.close()

    # ---------------------------------------------------------------- батарея
    print("Батарея")
    page, errors = open_island(browser)
    emit(page, "battery", {"hasBattery": True, "percent": 64, "charging": False})
    emit(page, "battery", {"hasBattery": True, "percent": 64, "charging": True}); settle(page, 700)
    check("подключение зарядки показывает активность", active_layer(page) == ["activity"] and size(page) == [236, 34], [active_layer(page), size(page)])
    check("в активности процент заряда", "64%" in page.inner_text("#act-right"))
    shot(page, "03_charging")
    settle(page, 3400)
    check("через 3.5 с активность исчезает", size(page) == [126, 34], size(page))
    check("нет ошибок JS", not errors, errors)
    page.close()

    # ---------------------------------------------------------------- музыка
    print("Музыка")
    page, errors = open_island(browser)
    emit(page, "media", MEDIA)
    emit(page, "media-art", {"key": "Spotify.exe|Нить|Тестовый исполнитель|Альбом", "url": art_png()})
    settle(page)
    check("играющая музыка — активность в свёрнутом виде", active_layer(page) == ["activity"], active_layer(page))
    check("в активности мини-обложка", page.locator("#act-left img.mini-art").count() == 1)
    check("и эквалайзер", page.locator("#act-right .eq.playing").count() == 1)
    shot(page, "04_music_activity")
    hover(page, True); settle(page)
    check("раскрытие открывает вкладку музыки", page.evaluate("document.querySelector('.tab.active').dataset.tab") == "music")
    check("название трека", page.inner_text("#m-title") == "Нить")
    check("исполнитель", page.inner_text("#m-artist") == "Тестовый исполнитель")
    check("приложение", page.inner_text("#m-app") == "Spotify")
    check("обложка показана", page.evaluate("!document.querySelector('#m-img').hidden"))
    accent = page.evaluate("getComputedStyle(document.querySelector('#island')).getPropertyValue('--accent')")
    check("цвет эквалайзера взят из обложки", accent.strip() not in ("", "#fff"), accent)
    t1 = page.inner_text("#m-pos"); settle(page, 1100); t2 = page.inner_text("#m-pos")
    check("прогресс идёт во время воспроизведения", t1 != t2, [t1, t2])
    check("оставшееся время со знаком минус", page.inner_text("#m-left").startswith("−"))
    shot(page, "05_music_expanded")

    page.click("#m-play")
    check("плей/пауза отправляет toggle", calls(page, "media_control")[-1][1]["action"] == "toggle")
    check("иконка сразу меняется на «плей»", page.evaluate("document.querySelector('#m-play-ico').getAttribute('href')") == "#i-play")
    page.click("#m-next")
    check("кнопка «вперёд»", calls(page, "media_control")[-1][1]["action"] == "next")
    page.click("#m-prev")
    check("кнопка «назад»", calls(page, "media_control")[-1][1]["action"] == "prev")
    box = page.locator("#m-progress").bounding_box()
    page.mouse.click(box["x"] + box["width"] * 0.5, box["y"] + box["height"] / 2)
    seek = calls(page, "media_control")[-1][1]
    check("клик по полоске перематывает к середине", seek["action"] == "seek" and abs(seek["positionMs"] - 100000) < 6000, seek)
    check("клики по кнопкам не сворачивают остров", size(page) == [400, 196], size(page))

    emit(page, "media", {**MEDIA, "title": "Другая песня", "durationMs": 0})
    settle(page, 300)
    check("смена трека обновляет название", page.inner_text("#m-title") == "Другая песня")
    check("старая обложка убрана до прихода новой", page.evaluate("document.querySelector('#m-img').hidden"))
    check("без длительности полоска скрыта", page.evaluate("document.querySelector('#m-bottom').classList.contains('no-time')"))
    hover(page, False); settle(page)
    emit(page, "media", {**MEDIA, "playing": False}); settle(page)
    check("на паузе остров сворачивается обратно", size(page) == [126, 34], size(page))
    check("нет ошибок JS", not errors, errors)
    page.close()

    # ---------------------------------------------------------------- таймер
    print("Таймер и секундомер")
    page, errors = open_island(browser)
    hover(page, True); settle(page)
    page.click('.tab[data-tab="timer"]')
    page.click('.chip[data-min="1"]')
    check("пресет 1 мин", page.inner_text("#t-setup-time") == "01:00")
    page.click("#t-plus"); page.click("#t-plus")
    check("плюс добавляет минуты", page.inner_text("#t-setup-time") == "03:00")
    page.click("#t-minus")
    check("минус убирает", page.inner_text("#t-setup-time") == "02:00")
    page.click("#t-main"); settle(page, 300)
    check("старт запускает отсчёт", page.inner_text("#t-main") == "Пауза" and page.is_visible("#timer-run"))
    shot(page, "06_timer_running")
    page.click("#t-main")
    paused = page.inner_text("#t-run-time"); settle(page, 1200)
    check("пауза останавливает время", page.inner_text("#t-run-time") == paused)
    page.click("#t-main")
    hover(page, False); settle(page)
    check("свёрнутый остров показывает таймер", active_layer(page) == ["activity"] and ":" in page.inner_text("#act-right"))
    shot(page, "07_timer_activity")
    # ускоряем: двигаем конец таймера в прошлое
    page.evaluate("timer.endAt = Date.now() - 1")
    settle(page, 700)
    check("по окончании — уведомление 360×76", size(page) == [360, 76] and active_layer(page) == ["alert"], [size(page), active_layer(page)])
    shot(page, "08_timer_alert")
    hover(page, True); settle(page, 400)
    check("во время уведомления наведение не раскрывает", size(page) == [360, 76])
    page.click("#alert-stop"); settle(page)
    check("кнопка закрывает уведомление", size(page)[1] != 76, size(page))
    hover(page, False); settle(page)

    hover(page, True); settle(page)
    page.click('.tab[data-tab="stopwatch"]')
    page.click("#sw-main"); settle(page, 400)
    check("секундомер идёт", page.inner_text("#sw-time") != "00:00,00")
    page.click("#sw-secondary")
    check("кнопка «Круг» записывает круг", page.inner_text("#sw-lap").startswith("Круг 1"))
    page.click("#sw-main")
    stopped = page.inner_text("#sw-time"); settle(page, 300)
    check("стоп останавливает", page.inner_text("#sw-time") == stopped)
    page.click("#sw-secondary")
    check("сброс обнуляет", page.inner_text("#sw-time") == "00:00,00" and page.inner_text("#sw-lap") == "")
    check("нет ошибок JS", not errors, errors)
    page.close()

    # ---------------------------------------------------------------- буфер
    print("Буфер обмена")
    page, errors = open_island(browser)
    items = [
        {"id": 3, "kind": "text", "text": "https://github.com/tauri-apps/tauri", "thumb": None, "width": 0, "height": 0, "chars": 35, "pinned": False, "at": int(time.time()*1000)},
        {"id": 2, "kind": "image", "text": "", "thumb": art_png(), "width": 1920, "height": 1080, "chars": 0, "pinned": False, "at": int(time.time()*1000) - 300000},
        {"id": 1, "kind": "text", "text": "func main() {\n\tfmt.Println(\"привет\")\n}", "thumb": None, "width": 0, "height": 0, "chars": 38, "pinned": True, "at": int(time.time()*1000) - 7200000},
    ]
    emit(page, "clipboard", {"items": items, "added": True}); settle(page, 500)
    check("новое копирование — «Скопировано» на свёрнутом острове", "Скопировано" in page.inner_text("#act-right"))
    shot(page, "09_clip_flash")
    settle(page, 1300)
    hover(page, True); settle(page)
    page.click('.tab[data-tab="clipboard"]'); settle(page)
    check("вкладка буфера вытягивает остров до 420×320", size(page) == [420, 320], size(page))
    check("три записи в списке", page.locator(".clip-row").count() == 3)
    check("у картинки подпись с размером", "1920×1080" in page.locator(".clip-row").nth(1).inner_text())
    check("время «2 ч назад»", "2 ч назад" in page.locator(".clip-row").nth(2).inner_text())
    check("кнопка «Очистить» видна", page.is_visible("#tab-clear"))
    shot(page, "10_clipboard")
    page.locator(".clip-row").nth(0).click()
    check("клик копирует запись обратно", calls(page, "clipboard_copy")[-1][1] == {"id": 3})
    page.wait_for_timeout(100)
    check("и показывает «Скопировано»", "Скопировано" in page.locator(".clip-row").nth(0).inner_text())
    page.locator(".clip-row").nth(1).hover()
    page.locator(".clip-row").nth(1).locator(".pin").click()
    check("закрепление", calls(page, "clipboard_pin")[-1][1] == {"id": 2})
    page.locator(".clip-row").nth(1).locator(".del").click()
    check("удаление", calls(page, "clipboard_delete")[-1][1] == {"id": 2})
    page.click("#tab-clear")
    check("очистка", len(calls(page, "clipboard_clear")) == 1)
    check("клики внутри не сворачивают остров", size(page) == [420, 320])
    emit(page, "clipboard", {"items": [], "added": False}); settle(page, 200)
    check("пустое состояние", page.is_visible("#clip-empty"))
    page.click('.tab[data-tab="music"]'); settle(page)
    check("назад на музыку — остров снова 400×196", size(page) == [400, 196], size(page))
    emit(page, "settings-changed", {**page.evaluate("window.__responses.get_settings"), "clipboardHistory": False})
    check("при выключенной истории вкладка скрыта", page.evaluate("document.querySelector('.tab[data-tab=clipboard]').hidden"))
    check("нет ошибок JS", not errors, errors)
    page.close()

    # ---------------------------------------------------------------- файлы
    print("Полка файлов")
    page, errors = open_island(browser)
    emit(page, "tauri://drag-enter", {"paths": ["C:\\a.pdf"], "position": {"x": 1, "y": 1}}); settle(page)
    check("перетаскивание файла раскрывает вкладку файлов", page.evaluate("document.querySelector('.tab.active').dataset.tab") == "files" and size(page) == [420, 320], size(page))
    check("видна зона «Отпустите…»", "dropping" in cls(page))
    shot(page, "11_drop_zone")
    emit(page, "tauri://drag-leave", None); settle(page, 300)
    check("уход файла убирает зону", "dropping" not in cls(page))
    emit(page, "tauri://drag-enter", {"paths": [], "position": {"x": 1, "y": 1}})
    emit(page, "tauri://drag-drop", {"paths": ["C:\\Users\\nik\\Отчёт.pdf", "C:\\Users\\nik\\photo.png"], "position": {"x": 1, "y": 1}})
    check("отпускание отправляет файлы в Rust", calls(page, "shelf_add")[-1][1] == {"paths": ["C:\\Users\\nik\\Отчёт.pdf", "C:\\Users\\nik\\photo.png"]})
    shelf_items = [
        {"id": 4, "path": "C:\\Users\\nik\\Отчёт по практике.pdf", "name": "Отчёт по практике.pdf", "ext": "pdf", "isDir": False, "size": 1843200, "exists": True, "added": 0},
        {"id": 3, "path": "C:\\Users\\nik\\photo.png", "name": "photo.png", "ext": "png", "isDir": False, "size": 523000, "exists": True, "added": 0},
        {"id": 2, "path": "C:\\Projects\\island", "name": "island", "ext": "", "isDir": True, "size": 0, "exists": True, "added": 0},
        {"id": 1, "path": "C:\\old\\main.go", "name": "main.go", "ext": "go", "isDir": False, "size": 2048, "exists": False, "added": 0},
    ]
    emit(page, "shelf", shelf_items); settle(page, 500)
    check("четыре плитки", page.locator(".tile").count() == 4)
    check("размер файла человеческий", "1.8 МБ" in page.locator(".tile").nth(0).inner_text())
    check("папка подписана", "Папка" in page.locator(".tile").nth(2).inner_text())
    check("пропавший файл помечен", "Не найден" in page.locator(".tile").nth(3).inner_text())
    check("картинка без доступа падает на обычный значок", page.locator(".tile").nth(1).locator(".doc").count() == 1)
    ic = calls(page, "shelf_icon")
    check("значки Windows запрошены для существующих файлов", sorted(c[1]["id"] for c in ic) == [2, 3, 4], ic)
    check("для пропавшего файла значок не запрашивается", all(c[1]["id"] != 1 for c in ic))
    emit(page, "shelf-icon", {"id": 4, "url": art_png()})
    emit(page, "shelf-icon", {"id": 2, "url": art_png()}); settle(page, 200)
    check("значок от Windows встал в плитку", page.locator('.tile[data-id="4"] img.sys-icon').count() == 1)
    check("и у папки тоже", page.locator('.tile[data-id="2"] img.sys-icon').count() == 1)
    shot(page, "12_shelf")
    emit(page, "settings-changed", {**page.evaluate("window.__responses.get_settings"), "fileIcons": "simple"}); settle(page, 200)
    check("режим «простые» убирает системные значки", page.locator("img.sys-icon").count() == 0)
    check("и показывает цветную плашку PDF", page.locator('.tile[data-id="4"] .doc').inner_text().lower() == "pdf")
    shot(page, "12b_shelf_simple")
    emit(page, "settings-changed", page.evaluate("window.__responses.get_settings")); settle(page, 200)
    check("назад к системным — значки из кэша, без повторного запроса", page.locator("img.sys-icon").count() == 2 and len(calls(page, "shelf_icon")) == 3)
    page.locator(".tile").nth(0).dblclick()
    check("двойной клик открывает файл", calls(page, "shelf_open")[-1][1] == {"id": 4})
    page.locator(".tile").nth(3).dblclick()
    check("пропавший файл не открывается", len(calls(page, "shelf_open")) == 1)
    page.locator(".tile").nth(0).hover(); page.locator(".tile").nth(0).locator(".reveal").click()
    check("«Показать в папке»", calls(page, "shelf_reveal")[-1][1] == {"id": 4})
    page.locator(".tile").nth(1).hover(); page.locator(".tile").nth(1).locator(".del").click()
    check("убрать с полки", calls(page, "shelf_remove")[-1][1] == {"id": 3})
    b = page.locator(".tile").nth(0).bounding_box()
    page.mouse.move(b["x"] + 30, b["y"] + 30); page.mouse.down()
    page.mouse.move(b["x"] + 33, b["y"] + 31)
    check("мелкое движение ещё не начинает перетаскивание", len(calls(page, "shelf_drag")) == 0)
    page.mouse.move(b["x"] + 60, b["y"] + 60); page.mouse.up()
    check("вытаскивание файла наружу", calls(page, "shelf_drag")[-1][1] == {"ids": [4]})
    emit(page, "tauri://drag-enter", {"paths": ["C:\\x"], "position": {"x": 1, "y": 1}})
    emit(page, "tauri://drag-drop", {"paths": ["C:\\Users\\nik\\Отчёт по практике.pdf"], "position": {"x": 1, "y": 1}})
    check("свой же файл, протащенный над островом, не добавляется снова", len(calls(page, "shelf_add")) == 1)
    emit(page, "shelf-drag-end", None)
    page.click("#tab-clear")
    check("очистить полку", len(calls(page, "shelf_clear")) == 1)
    check("нет ошибок JS", not errors, errors)
    page.close()

    # ---------------------------------------------------------------- громкость
    print("Громкость и яркость")
    page, errors = open_island(browser)
    emit(page, "volume", {"level": 0.62, "muted": False}); settle(page, 700)
    check("изменение громкости показывает индикатор 280×34", size(page) == [280, 34] and active_layer(page) == ["osd"], [size(page), active_layer(page)])
    check("процент громкости", page.inner_text("#osd-val") == "62")
    shot(page, "13_volume")
    emit(page, "volume", {"level": 0.62, "muted": True}); settle(page, 200)
    check("без звука — перечёркнутый динамик", page.evaluate("document.querySelector('#osd-ico-use').getAttribute('href')") == "#i-speaker-off")
    settle(page, 2200)
    check("индикатор сам исчезает", size(page) == [126, 34], size(page))
    emit(page, "brightness", 45); settle(page, 600)
    check("яркость показывает солнце и значение", page.inner_text("#osd-val") == "45" and page.evaluate("document.querySelector('#osd-ico-use').getAttribute('href')") == "#i-sun")
    shot(page, "14_brightness")
    settle(page, 1800)
    page.mouse.move(240, 17)
    page.mouse.wheel(0, -100); settle(page, 100)
    sv = calls(page, "set_volume")
    check("колесо вверх прибавляет громкость на 2%", sv and abs(sv[-1][1]["level"] - 0.64) < 1e-6, sv[-1:])
    page.mouse.wheel(0, 100); page.mouse.wheel(0, 100); settle(page, 100)
    check("колесо вниз убавляет", abs(calls(page, "set_volume")[-1][1]["level"] - 0.60) < 1e-6, calls(page, "set_volume")[-1:])
    hover(page, True); settle(page)
    n = len(calls(page, "set_volume"))
    page.mouse.wheel(0, -100); settle(page, 100)
    check("в раскрытом острове колесо громкость не трогает", len(calls(page, "set_volume")) == n)
    check("нет ошибок JS", not errors, errors)
    page.close()

    # ---------------------------------------------------------------- настройки
    print("Окно настроек")
    page = browser.new_page(viewport={"width": 440, "height": 900})
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.add_init_script(MOCK)
    page.goto((SRC / "settings.html").as_uri()); settle(page, 300)
    check("переключатели заполнены из настроек", page.is_checked('[data-key="hideInFullscreen"]') and not page.is_checked('[data-key="autostart"]'))
    page.click('label:has([data-key="autostart"])'); settle(page, 100)
    saved = calls(page, "save_settings")
    check("переключатель автозапуска сохраняет", saved and saved[-1][1]["value"]["autostart"] is True, saved[-1:])
    page.click('.segmented[data-key="expandOnHover"] button[data-value="false"]'); settle(page, 100)
    check("режим «по клику» сохраняется", calls(page, "save_settings")[-1][1]["value"]["expandOnHover"] is False)
    check("режим «по клику» подсвечен", page.evaluate("document.querySelector('.segmented[data-key=expandOnHover] .active').dataset.value") == "false")
    page.click('.segmented[data-key="fileIcons"] button[data-value="simple"]'); settle(page, 100)
    check("выбор простых значков сохраняется", calls(page, "save_settings")[-1][1]["value"]["fileIcons"] == "simple")
    page.click('.segmented[data-key="fileIcons"] button[data-value="system"]'); settle(page, 100)
    check("и обратно «как в Windows»", calls(page, "save_settings")[-1][1]["value"]["fileIcons"] == "system")
    check("переключатель системного индикатора включён", page.is_checked('[data-key="hideSystemOsd"]'))
    page.click('label:has([data-key="hideSystemOsd"])'); settle(page, 100)
    check("его выключение сохраняется", calls(page, "save_settings")[-1][1]["value"]["hideSystemOsd"] is False)
    page.click('label:has([data-key="osd"])'); settle(page, 100)
    check("без своего индикатора скрытие системного недоступно", page.is_disabled('[data-key="hideSystemOsd"]'))
    page.click('label:has([data-key="osd"])'); settle(page, 100)
    check("и снова доступно, когда индикатор включён", not page.is_disabled('[data-key="hideSystemOsd"]'))
    page.fill('[data-key="topOffset"]', "20"); settle(page, 400)
    check("ползунок отступа сохраняется", calls(page, "save_settings")[-1][1]["value"]["topOffset"] == 20)
    check("подпись отступа", page.inner_text("#offset-value") == "20 px")
    check("статус «Сохранено»", page.inner_text("#status") == "Сохранено")
    page.evaluate("window.__responses.save_settings = new Error('не удалось включить автозапуск: доступ запрещён')")
    page.click('label:has([data-key="osd"])'); settle(page, 200)
    check("ошибка сохранения показывается", "доступ запрещён" in page.inner_text("#status"))
    check("и переключатель откатывается", page.is_checked('[data-key="osd"]'))
    page.click("#quit")
    check("кнопка выхода", len(calls(page, "quit")) == 1)
    page.screenshot(path=str(SHOTS / "15_settings.png"), full_page=True)
    page.emulate_media(color_scheme="dark"); settle(page, 100)
    page.screenshot(path=str(SHOTS / "16_settings_dark.png"), full_page=True)
    check("нет ошибок JS", not errors, errors)
    page.close()

    browser.close()

failed = [r for r in results if not r[1]]
print(f"\nИтого: {len(results) - len(failed)} из {len(results)} проверок прошли")
sys.exit(1 if failed else 0)
