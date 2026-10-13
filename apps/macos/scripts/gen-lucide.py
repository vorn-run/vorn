#!/usr/bin/env python3
"""Writes Sources/VornUI/LucideGlyphs.swift from the lucide-static SVGs the renderer's icons come from."""

import concurrent.futures
import pathlib
import re
import urllib.request

VERSION = "1.34.0"  # lucide-react in package.json
ICONS = {
    # Swift name: lucide file
    "circle": "circle", "clock": "clock", "eye": "eye", "circleCheck": "circle-check",
    "circleX": "circle-x", "plus": "plus", "ellipsis": "ellipsis", "pencil": "pencil",
    "trash2": "trash-2", "archive": "archive", "archiveRestore": "archive-restore",
    "rotateCcw": "rotate-ccw", "fileCode": "file-code", "play": "play", "terminal": "terminal",
    "slidersHorizontal": "sliders-horizontal", "listTodo": "list-todo", "chevronDown": "chevron-down",
    "chevronRight": "chevron-right", "x": "x", "maximize2": "maximize-2", "paperclip": "paperclip",
    "folderGit2": "folder-git-2", "gitBranch": "git-branch", "calendar": "calendar",
    "imagePlus": "image-plus", "save": "save", "check": "check", "folder": "folder", "bot": "bot",
    "zap": "zap", "clipboardList": "clipboard-list", "code": "code", "globe": "globe",
    "database": "database", "server": "server", "smartphone": "smartphone", "package": "package",
    "cpu": "cpu", "cloud": "cloud", "shield": "shield", "gamepad2": "gamepad-2", "music": "music",
    "image": "image", "bookOpen": "book-open", "flaskConical": "flask-conical", "rocket": "rocket",
    "panelLeft": "panel-left",
    "monitor": "monitor",
    "triangleAlert": "triangle-alert",
    "workflow": "workflow",
    "chevronsRight": "chevrons-right",
    "layers": "layers",
    "folderPlus": "folder-plus",
    "listFilter": "list-filter",
    "circleHelp": "circle-help",
    "settings": "settings",
    "folderTree": "folder-tree",
    "folderOpen": "folder-open",
    "squareTerminal": "square-terminal",
    "minus": "minus",
    "minimize2": "minimize-2",
    "arrowUp": "arrow-up",
    "lightbulb": "lightbulb",
    "user": "user",
    "briefcase": "briefcase",
    "house": "house",
    "building2": "building-2",
    "heart": "heart",
    "star": "star",
    "sparkles": "sparkles",
    "chevronLeft": "chevron-left",
    "gripVertical": "grip-vertical",
    "arrowDown": "arrow-down",
    "keyboard": "keyboard",
    "circleCheckBig": "circle-check-big",
    "copy": "copy",
    "cornerDownLeft": "corner-down-left",
    "download": "download",
    "externalLink": "external-link",
    "fileKey": "file-key",
    "gitCommitHorizontal": "git-commit-horizontal",
    "hardDrive": "hard-drive",
    "info": "info",
    "key": "key",
    "keyRound": "key-round",
    "listTree": "list-tree",
    "loaderCircle": "loader-circle",
    "messageSquare": "message-square",
    "penLine": "pen-line",
    "refreshCw": "refresh-cw",
    "search": "search",
    "send": "send",
    "squareArrowOutUpRight": "square-arrow-out-up-right",
    "undo2": "undo-2",
    "upload": "upload",
    "wifi": "wifi",
    "heading1": "heading-1",
    "heading2": "heading-2",
    "heading3": "heading-3",
    "list": "list",
    "listOrdered": "list-ordered",
    "squareCheckBig": "square-check-big",
    "quote": "quote",
    "bold": "bold",
    "italic": "italic",
    "strikethrough": "strikethrough",
    "link": "link",
    "unlink": "unlink",
    "activity": "activity",
    "alignVerticalSpaceAround": "align-vertical-space-around",
    "arrowLeft": "arrow-left",
    "arrowRightLeft": "arrow-right-left",
    "braces": "braces",
    "codeXml": "code-xml",
    "eyeOff": "eye-off",
    "filePlusCorner": "file-plus-corner",
    "gitFork": "git-fork",
    "hand": "hand",
    "historyClock": "rotate-ccw-clock",
    "inbox": "inbox",
    "listPlus": "list-plus",
    "lock": "lock",
    "logIn": "log-in",
    "maximize": "maximize",
    "plug": "plug",
    "power": "power",
    "repeatIcon": "repeat",
    "replace": "replace",
    "settings2": "settings-2",
    "split": "split",
    "square": "square",
    "stepForward": "step-forward",
    "arrowRight": "arrow-right",
    "rotateCw": "rotate-cw",
    "imageDown": "image-down",
    "scan": "scan",
    "messageSquarePlus": "message-square-plus",
    "mousePointerClick": "mouse-pointer-click",
    "alignLeft": "align-left",
    "fileText": "file-text",
    "shapes": "shapes",
    "filter": "filter",
    "arrowUpDown": "arrow-up-down",
    "layoutDashboard": "layout-dashboard",
}
# Inline glyphs from TaskToolbar.tsx and OptionRow.tsx rather than lucide.
INLINE = {
    "viewList": '<line x1="8" y1="6" x2="21" y2="6" /><line x1="8" y1="12" x2="21" y2="12" />'
    '<line x1="8" y1="18" x2="21" y2="18" /><line x1="3" y1="6" x2="3.01" y2="6" />'
    '<line x1="3" y1="12" x2="3.01" y2="12" /><line x1="3" y1="18" x2="3.01" y2="18" />',
    "viewKanban": '<rect x="3" y="3" width="18" height="18" rx="2" /><path d="M9 3v18" /><path d="M15 3v18" />',
    "optionCheck": '<polyline points="20 6 9 17 4 12" />',
    "layoutGrid": '<rect x="3" y="3" width="7" height="7" /><rect x="14" y="3" width="7" height="7" />'
    '<rect x="3" y="14" width="7" height="7" /><rect x="14" y="14" width="7" height="7" />',
    "layoutTabs": '<rect x="3" y="3" width="18" height="18" rx="2" /><path d="M3 9h18" /><path d="M9 3v6" />',
    # SettingsPage.tsx's own nav glyphs.
    "navAppearance": '<circle cx="12" cy="12" r="5" /><path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />',
    "navGeneral": '<circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83 0 2 2 0 010-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 112.83-2.83l.06.06a1.65 1.65 0 001.82.33H9a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 112.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z" />',
    "navUpdates": '<path d="M12 3v12" /><path d="M7 10l5 5 5-5" /><path d="M4 20h16" />',
    "navNotifications": '<path d="M18 8A6 6 0 006 8c0 7-3 9-3 9h18s-3-2-3-9" /><path d="M13.73 21a2 2 0 01-3.46 0" />',
    "navWorktrees": '<circle cx="6" cy="5" r="2.5" /><circle cx="18" cy="5" r="2.5" /><circle cx="12" cy="19" r="2.5" /><path d="M6 7.5v3a2 2 0 002 2h8a2 2 0 002-2v-3M12 12.5v4" />',
    "navAgents": '<polyline points="4 17 10 11 4 5" /><line x1="12" y1="19" x2="20" y2="19" />',
    "navSSH": '<rect x="2" y="2" width="20" height="8" rx="2" /><rect x="2" y="14" width="20" height="8" rx="2" /><circle cx="6" cy="6" r="1" /><circle cx="6" cy="18" r="1" />',
    "navMcp": '<path d="M12 2v4m0 12v4M2 12h4m12 0h4" /><circle cx="12" cy="12" r="4" />',
    "navPlugins": '<path d="M9 17H7A5 5 0 017 7h2" /><path d="M15 7h2a5 5 0 010 10h-2" /><line x1="8" y1="12" x2="16" y2="12" />',
    "navKeys": '<circle cx="8" cy="12" r="4" /><line x1="12" y1="12" x2="21" y2="12" /><line x1="18" y1="12" x2="18" y2="16" />',
    "navNetwork": '<circle cx="12" cy="12" r="10" /><line x1="2" y1="12" x2="22" y2="12" /><path d="M12 2a15.3 15.3 0 014 10 15.3 15.3 0 01-4 10 15.3 15.3 0 01-4-10 15.3 15.3 0 014-10z" />',
    "navAbout": '<circle cx="12" cy="12" r="10" /><line x1="12" y1="16" x2="12" y2="12" /><line x1="12" y1="8" x2="12.01" y2="8" />',
    "navBack": '<polyline points="15 18 9 12 15 6" />',
}

# Lucide component names a project or workspace stores in config, as the sidebar resolves them.
COMPONENTS = {
    "Folder": "folder",
    "FolderGit2": "folderGit2",
    "Code": "code",
    "Globe": "globe",
    "Database": "database",
    "Server": "server",
    "Smartphone": "smartphone",
    "Package": "package",
    "FileCode": "fileCode",
    "Terminal": "terminal",
    "Cpu": "cpu",
    "Cloud": "cloud",
    "Shield": "shield",
    "Zap": "zap",
    "Workflow": "workflow",
    "Gamepad2": "gamepad2",
    "Music": "music",
    "Image": "image",
    "BookOpen": "bookOpen",
    "FlaskConical": "flaskConical",
    "Rocket": "rocket",
    "Play": "play",
    "User": "user",
    "Briefcase": "briefcase",
    "Home": "house",
    "House": "house",
    "Building2": "building2",
    "Heart": "heart",
    "Star": "star",
    "Sparkles": "sparkles",
    "Layers": "layers",
}

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "Sources/VornUI/LucideGlyphs.swift"


def fetch(file: str) -> str:
    url = f"https://unpkg.com/lucide-static@{VERSION}/icons/{file}.svg"
    for attempt in range(4):
        try:
            with urllib.request.urlopen(url, timeout=30) as r:
                return r.read().decode()
        except OSError:
            if attempt == 3:
                raise


def attrs(tag: str) -> dict:
    return dict(re.findall(r'([\w-]+)="([^"]*)"', tag))


def elements(svg: str) -> list[str]:
    out = []
    for tag in re.findall(r"<(path|circle|rect|line|polyline|polygon|ellipse)\b([^>]*)/?>", svg):
        kind, rest = tag
        a = attrs(rest)
        n = lambda k, d="0": f"{float(a.get(k, d)):g}"
        if kind == "path":
            out.append(f'.path("{a["d"]}")')
        elif kind == "circle":
            out.append(f".circle(cx: {n('cx')}, cy: {n('cy')}, r: {n('r')})")
        elif kind == "ellipse":
            out.append(f".ellipse(cx: {n('cx')}, cy: {n('cy')}, rx: {n('rx')}, ry: {n('ry')})")
        elif kind == "rect":
            rx = f'{float(a.get("rx", a.get("ry", "0"))):g}'
            out.append(f".rect(x: {n('x')}, y: {n('y')}, width: {n('width')}, height: {n('height')}, rx: {rx})")
        elif kind == "line":
            out.append(f".line(x1: {n('x1')}, y1: {n('y1')}, x2: {n('x2')}, y2: {n('y2')})")
        elif kind in ("polyline", "polygon"):
            pts = [float(v) for v in re.split(r"[\s,]+", a["points"].strip())]
            pairs = ", ".join(f"({pts[i]:g}, {pts[i + 1]:g})" for i in range(0, len(pts), 2))
            out.append(f".poly([{pairs}], closed: {'true' if kind == 'polygon' else 'false'})")
    return out


def main() -> None:
    with concurrent.futures.ThreadPoolExecutor(4) as pool:
        svgs = dict(zip(ICONS, pool.map(fetch, ICONS.values())))
    svgs.update(INLINE)
    lines = [
        "// Generated by scripts/gen-lucide.py from lucide-static " + VERSION + " (ISC). Do not edit.",
        "",
        "public extension LucideGlyph {",
    ]
    for name in sorted(svgs):
        body = ", ".join(elements(svgs[name]))
        lines.append(f"    static let {name} = LucideGlyph([{body}])")
    lines.append("")
    lines.append("    /// The icon a project or workspace names (a lucide component name, as stored in config).")
    lines.append("    init?(componentName: String) {")
    lines.append("        switch componentName {")
    for component, name in COMPONENTS.items():
        lines.append(f'        case "{component}": self = .{name}')
    lines.append("        default: return nil")
    lines.append("        }")
    lines.append("    }")
    lines.append("}")
    OUT.write_text("\n".join(lines) + "\n")
    print(f"wrote {OUT} ({len(svgs)} glyphs)")


if __name__ == "__main__":
    main()
