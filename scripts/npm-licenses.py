#!/usr/bin/env python3
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PACKAGES = ["frontend/admin", "frontend/shared", "frontend/webmail"]
LICENSE_NAMES = ("LICENSE", "LICENCE", "LICENSE.md", "LICENSE.txt", "LICENCE.md", "COPYING")


def read_json(path):
    try:
        with open(path, encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, ValueError):
        return None


def resolve(name, start):
    directory = start
    while True:
        candidate = os.path.realpath(os.path.join(directory, "node_modules", name))
        if os.path.isfile(os.path.join(candidate, "package.json")):
            return candidate
        parent = os.path.dirname(directory)
        if parent == directory or len(directory) < len(ROOT):
            return None
        directory = parent


def license_id(manifest):
    value = manifest.get("license")
    if isinstance(value, dict):
        return value.get("type", "UNKNOWN")
    if isinstance(value, list):
        return " OR ".join(entry.get("type", "UNKNOWN") for entry in value)
    if value:
        return value
    legacy = manifest.get("licenses")
    if isinstance(legacy, list) and legacy:
        return " OR ".join(entry.get("type", "UNKNOWN") for entry in legacy)
    return "UNKNOWN"


def license_text(directory):
    for entry in sorted(os.listdir(directory)):
        if entry.upper() in [name.upper() for name in LICENSE_NAMES]:
            path = os.path.join(directory, entry)
            if os.path.isfile(path):
                with open(path, encoding="utf-8", errors="replace") as handle:
                    return handle.read().strip()
    return ""


def collect():
    found = {}
    seen = set()
    queue = []
    for package in PACKAGES:
        manifest = read_json(os.path.join(ROOT, package, "package.json"))
        if not manifest:
            continue
        for name in manifest.get("dependencies", {}):
            queue.append((name, os.path.join(ROOT, package)))

    while queue:
        name, start = queue.pop()
        if name.startswith("@irixmail/"):
            continue
        directory = resolve(name, start)
        if not directory:
            continue
        manifest = read_json(os.path.join(directory, "package.json"))
        if not manifest:
            continue
        key = f"{name}@{manifest.get('version', '0.0.0')}"
        if key in seen:
            continue
        seen.add(key)
        found[key] = {
            "name": name,
            "version": manifest.get("version", "0.0.0"),
            "license": license_id(manifest),
            "text": license_text(directory),
        }
        for child in manifest.get("dependencies", {}):
            queue.append((child, directory))
    return found


def main():
    packages = collect()
    if not packages:
        sys.exit("no frontend dependencies resolved; run `bun install` first")
    out = []
    for key in sorted(packages, key=lambda item: item.lower()):
        entry = packages[key]
        link = f"https://www.npmjs.com/package/{entry['name']}"
        out.append(f"### {entry['name']} {entry['version']}\n")
        out.append(f"License: {entry['license']} — [source]({link})\n")
        if entry["text"]:
            out.append("```\n" + entry["text"] + "\n```\n")
        out.append("")
    sys.stdout.write("\n".join(out))


if __name__ == "__main__":
    main()
