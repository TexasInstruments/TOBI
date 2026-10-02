#!/usr/bin/env python3
"""Build TOBI's static board guide with Python's standard library."""

import argparse
import html
import json
import re
import shutil
from pathlib import Path
from urllib.parse import urlparse

SOURCE = Path(__file__).resolve().parent
REPO = SOURCE.parent
BASE_URL = "https://texasinstruments.github.io/TOBI/"


def escape(value):
    return html.escape(str(value), quote=True)


def load_boards():
    data = json.loads((SOURCE / "boot-guide.json").read_text())
    boards = data["boards"]
    photos = json.loads((SOURCE / "board-photos.json").read_text())
    catalog = json.loads((REPO / "catalog.json").read_text())
    expected = {entry["id"]: entry["name"] for entry in catalog["devices"]}
    assert data["schema_version"] == 1
    assert len(boards) == len(expected)
    assert {board["board_id"] for board in boards} == set(expected)
    assert photos["schema_version"] == 1
    assert set(photos["boards"]) == set(expected)
    for board in boards:
        board_id = board["board_id"]
        assert re.fullmatch(r"[a-z0-9-]+", board_id), board_id
        assert board["name"] == expected[board_id]
        assert board["url"] == BASE_URL + "boards/" + board_id + "/"
        assert board["summary"] and board["steps"] and board["sources"]
        assert board["emmc_status"] in {"filesystem", "boot0", "none"}
        board["photo"] = photos["boards"][board_id]
        for key in ("image_url", "source_url"):
            url = urlparse(board["photo"][key])
            assert url.scheme == "https" and url.hostname == "www.ti.com", url
        assert board["photo"]["alt"]
        for bank in board.get("switches", []):
            assert len(bank["states"]) == len(bank["bits"])
            assert all(state in ("ON", "OFF") for state in bank["states"])
            for bit in bank["bits"]:
                bit_number(bit)
        for source in board["sources"]:
            assert source["url"].startswith("https://")
    return boards


def bit_number(signal):
    """Use global BOOTMODE indices, or the SK-AM69 selector indices."""
    match = re.fullmatch(r"(?:B|Select )(\d+)", signal)
    assert match, signal
    return int(match[1])


def diagram_dimensions(board):
    if board["board_id"] == "sk-am69":
        return 440, 300
    return 760, 590 if len(board.get("switches", [])) > 2 else 440


def page(title, description, body, prefix, canonical):
    return f'''<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="description" content="{escape(description)}">
  <meta name="theme-color" content="#b40000">
  <title>{escape(title)} · TOBI</title>
  <link rel="canonical" href="{escape(canonical)}">
  <link rel="stylesheet" href="{prefix}assets/style.css">
</head>
<body>
  <a class="skip" href="#main">Skip to instructions</a>
  <header class="site-header"><div class="header-inner">
    <a class="brand" href="{prefix}index.html">TOBI<span>TI Out of Box Installer</span></a>
    <a href="https://github.com/TexasInstruments/TOBI">Source on GitHub ↗</a>
  </div></header>
  <main id="main">{body}</main>
  <footer>TOBI board boot guides · Check the model and revision printed on your board.<br>
    <a href="https://github.com/TexasInstruments/TOBI/issues">Report a correction</a></footer>
</body>
</html>
'''


def diagram(board):
    """Centered illustrations labeled with zero-based signal indices."""
    canvas_width, height = diagram_dimensions(board)
    center = canvas_width / 2
    parts = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {canvas_width} {height}" role="img">',
             f'<title>{escape(board["name"])} boot configuration</title>',
             f'<rect width="{canvas_width}" height="{height}" rx="20" fill="#f3f5f5"/>',
             f'<text x="{center}" y="42" text-anchor="middle" font-family="sans-serif" font-size="23" font-weight="700" fill="#222">{escape(board["name"])}</text>']
    if board["emmc_status"] == "none":
        parts.extend([
            '<rect x="220" y="95" width="320" height="215" rx="20" fill="#173f37"/>',
            '<rect x="310" y="118" width="140" height="140" rx="12" fill="#dde8e4"/>',
            '<text x="380" y="192" text-anchor="middle" font-family="sans-serif" font-size="30" font-weight="700" fill="#173f37">microSD</text>',
            '<text x="380" y="354" text-anchor="middle" font-family="sans-serif" font-size="22" font-weight="700" fill="#222">No onboard eMMC</text>',
            '<text x="380" y="387" text-anchor="middle" font-family="sans-serif" font-size="17" fill="#444">Keep using SD or compatible external storage.</text>',
        ])
    elif board["board_id"] == "beagleplay":
        parts.extend([
            '<rect x="150" y="90" width="460" height="230" rx="20" fill="#173f37"/>',
            '<rect x="185" y="130" width="140" height="135" rx="12" fill="#cbd1d1"/>',
            '<text x="255" y="290" text-anchor="middle" font-family="sans-serif" font-size="17" fill="white">microSD slot</text>',
            '<circle cx="463" cy="190" r="45" fill="#303635" stroke="#e8ecea" stroke-width="10"/>',
            '<text x="463" y="267" text-anchor="middle" font-family="sans-serif" font-size="22" fill="white">USR</text>',
            '<text x="380" y="359" text-anchor="middle" font-family="sans-serif" font-size="23" font-weight="700" fill="#222">Remove SD · Leave USR released</text>',
            '<text x="380" y="391" text-anchor="middle" font-family="sans-serif" font-size="17" fill="#444">Button illustration; see the board manual for its location.</text>',
        ])
    else:
        banks = board.get("switches", [])
        count = len(banks)
        row_height = (height - 140) / max(count, 1)
        for row, bank in enumerate(banks):
            y = 96 + row * row_height
            n = len(bank["states"])
            step = 64
            bank_width = n * step + 24
            bank_x = (canvas_width - bank_width) / 2
            parts.append(f'<text x="{center}" y="{y - 20}" text-anchor="middle" font-family="sans-serif" font-size="21" font-weight="700" fill="#222">{escape(bank["bank"])}</text>')
            parts.append(f'<text x="{bank_x}" y="{y - 7}" font-family="sans-serif" font-size="12" font-weight="700" fill="#8d0000">ON ↑</text>')
            parts.append(f'<rect x="{bank_x}" y="{y}" width="{bank_width}" height="78" rx="8" fill="#8d0000"/>')
            for i, (state, bit) in enumerate(zip(bank["states"], bank["bits"])):
                width = 48
                x = bank_x + 12 + i * step + (step - width) / 2
                parts.append(f'<rect x="{x}" y="{y + 10}" width="{width}" height="48" rx="4" fill="#e4a5a5"/>')
                slider_y = y + (12 if state == "ON" else 36)
                parts.append(f'<rect x="{x + 3}" y="{slider_y}" width="{width - 6}" height="20" rx="3" fill="#fff"/>')
                parts.append(f'<text x="{x + width / 2}" y="{y + 73}" text-anchor="middle" font-family="sans-serif" font-size="16" font-weight="700" fill="white">{bit_number(bit)}</text>')
                parts.append(f'<text x="{x + width / 2}" y="{y + 96}" text-anchor="middle" font-family="sans-serif" font-size="14" font-weight="700" fill="#222">{state}</text>')
                parts.append(f'<text x="{x + width / 2}" y="{y + 112}" text-anchor="middle" font-family="sans-serif" font-size="12" fill="#555">{escape(bit)}</text>')
        kind = "Selector" if board["board_id"] == "sk-am69" else "BOOTMODE"
        reminder = "Only physical SW2.1–SW2.3 shown · Leave SW2.4 unchanged." if board["board_id"] == "sk-am69" else "Diagram labels are bit numbers; printed switch numbers start at 1."
        parts.append(f'<text x="{center}" y="{height - 34}" text-anchor="middle" font-family="sans-serif" font-size="12" fill="#444">{reminder}</text>')
        parts.append(f'<text x="{center}" y="{height - 17}" text-anchor="middle" font-family="sans-serif" font-size="14" fill="#444">{kind} bit numbers · Match your board\'s ON mark.</text>')
    parts.append('</svg>')
    return "\n".join(parts) + "\n"


def board_body(board):
    diagram_width, diagram_height = diagram_dimensions(board)
    figure_class = "switch-figure compact" if board["board_id"] == "sk-am69" else "switch-figure"
    steps = "".join(f"<li>{escape(step)}</li>" for step in board["steps"])
    notes = "".join(f"<li>{escape(note)}</li>" for note in board.get("notes", []))
    sources = "".join(f'<li><a href="{escape(source["url"])}">{escape(source["title"])} ↗</a></li>' for source in board["sources"])
    table = ""
    for bank in board.get("switches", []):
        numbers = "".join(f'<th scope="col">{bit_number(bit)}</th>' for bit in bank["bits"])
        physical = "".join(f"<td>{i + 1}</td>" for i in range(len(bank["states"])))
        states = "".join(f'<td class="{state.lower()}">{state}</td>' for state in bank["states"])
        bits = "".join(f'<td>{escape(bit)}</td>' for bit in bank["bits"])
        kind = "Selector bit" if board["board_id"] == "sk-am69" else "BOOTMODE bit"
        table += f'<div class="table-scroll"><table><caption>{escape(bank["bank"])} · bit-to-switch mapping</caption><thead><tr><th scope="col">{kind}</th>{numbers}</tr></thead><tbody><tr><th scope="row">Position</th>{states}</tr><tr><th scope="row">Printed switch</th>{physical}</tr><tr><th scope="row">Signal</th>{bits}</tr></tbody></table></div>'
    numbering = ""
    if board.get("switches"):
        kind = "selector" if board["board_id"] == "sk-am69" else "BOOTMODE"
        numbering = f'<p class="small numbering">The diagram uses zero-based {kind} bit numbers. The steps above use the physical switch numbers printed on the board, starting at 1. Use the tables below to match them.</p>'
        if board["board_id"] == "sk-am69":
            numbering += '<p class="small">Only SW2 positions 1–3 are shown. Leave physical switch SW2.4 unchanged.</p>'
    photo = board["photo"]
    recovery = "".join(f"<li>{escape(step)}</li>" for step in board.get("recovery_steps", []))
    return f'''<nav class="breadcrumb"><a href="../../index.html">All boards</a><span>/</span>{escape(board["name"])}</nav>
<section class="hero"><p class="eyebrow">After flashing</p><h1>{escape(board["name"])}</h1>
<p class="lead">{escape(board["summary"])}</p><span class="badge">{escape(board["mode_title"])}</span></section>
<figure class="board-photo"><div class="photo-frame"><img src="{escape(photo["image_url"])}" alt="{escape(photo["alt"])}" decoding="async"></div><figcaption>Board photo from <a href="{escape(photo["source_url"])}">Texas Instruments ↗</a>. Check the model and revision on your board.</figcaption></figure>
<div class="notice"><strong>Disconnect power first.</strong> Boot straps are sampled during power-on. Remove the recovery SD only after flashing has finished.</div>
<section class="instructions"><h2>Start the installed image</h2><ol class="steps">{steps}</ol></section>
<section><h2>Boot configuration at a glance</h2>{numbering}<figure class="{figure_class}"><img class="switch-diagram" src="../../assets/{escape(board["board_id"])}-boot.svg" alt="{escape(board["name"])} boot configuration; switch states and bit numbers are listed in the tables below." width="{diagram_width}" height="{diagram_height}"><figcaption>Original TOBI diagram based on the official sources below. Rotate to match the ON mark printed on your board.</figcaption></figure>{table}</section>
<section><h2>Before the next boot</h2><ul class="notes">{notes}</ul></section>
<section class="recovery"><h2>If the board does not boot</h2><ol>{recovery}</ol></section>
<section><h2>Official references</h2><ul class="references">{sources}</ul><p class="small">Reviewed {escape(board["verified_on"])}. Physical-board validation is still required for the installed image and your board revision.</p></section>
<div class="page-end"><a class="button" href="../../index.html">Choose another board</a><a href="{escape(board["url"])}">Permanent link to this guide</a></div>'''


def build(destination):
    boards = load_boards()
    destination.mkdir(parents=True, exist_ok=True)
    assets = destination / "assets"
    assets.mkdir(exist_ok=True)
    shutil.copyfile(SOURCE / "style.css", assets / "style.css")
    cards = ""
    for board in boards:
        board_id = board["board_id"]
        folder = destination / "boards" / board_id
        folder.mkdir(parents=True, exist_ok=True)
        (assets / (board_id + "-boot.svg")).write_text(diagram(board))
        (folder / "index.html").write_text(page(board["name"] + " boot guide", board["summary"], board_body(board), "../../", board["url"]))
        cards += f'<a class="board-card" href="boards/{escape(board_id)}/"><img class="card-photo" src="{escape(board["photo"]["image_url"])}" alt="{escape(board["photo"]["alt"])}" loading="lazy" decoding="async"><span class="badge">{escape(board["mode_title"])}</span><h2>{escape(board["name"])}</h2><p>{escape(board["summary"])}</p><span class="card-link">View boot guide →</span></a>'
    home = f'''<section class="hero"><p class="eyebrow">TOBI board guides</p><h1>Flash complete.<br>Set the next boot.</h1><p class="lead">Choose your board for its boot switches, diagrams, and power-on instructions.</p></section>
<div class="notice"><strong>Check the exact board name.</strong> Switch banks and boot modes differ between kits. Some supported boards use microSD and have no onboard eMMC.</div>
<section class="board-grid" aria-label="Supported boards">{cards}</section>
<section class="about"><h2>From TOBI to this guide</h2><p>After a successful eMMC installation, TOBI shows the settings for the detected board. Press <kbd>G</kbd> to display a QR code and open that board's guide on your phone.</p><p>Settings here describe the boot path prepared by TOBI. A third-party image can require a different bootloader layout. Keep your recovery SD card until the installed image boots successfully.</p><p class="small">Board photos from Texas Instruments. Each guide links to the board's TI product page.</p></section>'''
    (destination / "index.html").write_text(page("Board boot guides", "Board-specific boot switches and illustrated eMMC instructions for TOBI.", home, "", BASE_URL))
    (destination / ".nojekyll").touch()
    (destination / "404.html").write_text(page("Guide not found", "Find your TOBI board boot guide.", '<section class="hero"><h1>Find your board guide</h1><p class="lead">This link does not identify a supported board.</p><a class="button" href="/TOBI/">Choose your board</a></section>', "/TOBI/", BASE_URL))
    print(f"Built {len(boards)} board pages and switch diagrams in {destination}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=REPO / "out" / "docs-site")
    args = parser.parse_args()
    build(args.output.resolve())
