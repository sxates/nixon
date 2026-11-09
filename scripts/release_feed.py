#!/usr/bin/env python3
"""release_feed.py: the pure parts of release.sh's feed publishing (specs/0086).

Standard library only. release.sh calls it to build the updater manifests, the release
notes and the version comparisons, so that logic can be tested here instead of living in
inline heredocs. This is release tooling, not an app runtime dependency.

Subcommands:
  notes --changelog F --upto X [--after V] [--out F]
      The notes rule: every "## [A.B.C]" section with V < A.B.C <= X, newest first, each
      with its "### Internal" subsection removed. A single section is printed as its body
      only (exactly what release.sh has always published). When several are combined, each
      body is preceded by a plain "Nixon A.B.C" line. A missing or empty V means "just X".
      Falls back to "Nixon vX" when nothing is left.
  manifest --version X --notes F --sig F --url U --out F [--dry-run] [--pub-date D]
      The updater manifest (the shape of updates/latest.json). With --dry-run, a missing
      signature file becomes a "<signature>" placeholder instead of an error.
  newer A [B]
      Exit 0 if A > B as X.Y.Z, 1 if not, 2 if A is not X.Y.Z. A missing, empty or
      "<unknown>" B counts as older.
  keyid-match --pubkey P --sig F
      Exit 0 if the minisign key id in the .sig file F matches the base64 pubkey P,
      1 if they differ, 2 if either cannot be parsed.
  version [--file F]
      Print the "version" field of a manifest read from F (default stdin). Exit 2 if it is
      not a manifest with an X.Y.Z version.
  --self-test
      Run the built-in cases.
"""

import argparse
import base64
import datetime
import json
import os
import re
import sys
import tempfile

SEMVER_RE = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")
SECTION_RE = re.compile(r"^## \[(\d+\.\d+\.\d+)\]")
UNKNOWN = ("", "<unknown>", "none", "null")


def parse_semver(v):
    m = SEMVER_RE.match((v or "").strip())
    return tuple(int(p) for p in m.groups()) if m else None


def is_newer(a, b):
    """True if a > b. A missing/unknown b counts as older. Raises ValueError on a bad a."""
    pa = parse_semver(a)
    if pa is None:
        raise ValueError(f"not an X.Y.Z version: {a!r}")
    if b is None or b.strip() in UNKNOWN:
        return True
    pb = parse_semver(b)
    if pb is None:
        raise ValueError(f"not an X.Y.Z version: {b!r}")
    return pa > pb


def changelog_sections(text):
    """{version: body_lines} for every "## [X.Y.Z]" section, with ### Internal removed.

    Mirrors the awk release.sh used before this helper: the section starts after its
    heading line and ends at the next "## [" heading; "### Internal" runs until the next
    "### " heading.
    """
    sections = {}
    cur = None
    skip = False
    for line in text.splitlines():
        if line.startswith("## ["):
            m = SECTION_RE.match(line)
            cur = m.group(1) if m else None
            skip = False
            if cur is not None and cur not in sections:
                sections[cur] = []
            elif cur is not None:
                cur = None  # a duplicate heading: keep the first, as the awk did
            continue
        if cur is None:
            continue
        if re.match(r"^### Internal\s*$", line):
            skip = True
            continue
        if skip and line.startswith("### "):
            skip = False
        if not skip:
            sections[cur].append(line)
    return sections


def build_notes(changelog_text, upto, after=None):
    if parse_semver(upto) is None:
        raise ValueError(f"not an X.Y.Z version: {upto!r}")
    sections = changelog_sections(changelog_text)
    if after is None or after.strip() in UNKNOWN:
        chosen = [upto] if upto in sections else []
    else:
        chosen = [v for v in sections if is_newer(v, after) and not is_newer(v, upto)]
        chosen.sort(key=parse_semver, reverse=True)
    bodies = [(v, "".join(line + "\n" for line in sections[v])) for v in chosen]
    bodies = [(v, b) for v, b in bodies if b.strip()]
    if not bodies:
        return f"Nixon v{upto}\n"
    if len(bodies) == 1:
        return bodies[0][1]
    return "\n".join(f"Nixon {v}\n\n{b.strip()}\n" for v, b in bodies)


def build_manifest(version, notes, sig, url, pub_date=None):
    if pub_date is None:
        pub_date = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    return {
        "version": version,
        "notes": notes.strip(),
        "pub_date": pub_date,
        "platforms": {"darwin-aarch64": {
            "signature": sig,
            "url": url}},
    }


def write_manifest(out, manifest):
    # indent=2 and no trailing newline: byte-identical to what release.sh emitted before.
    with open(out, "w") as fh:
        json.dump(manifest, fh, indent=2)


def keyid(b64):
    text = base64.b64decode(b64).decode()
    line = [l for l in text.splitlines() if l and not l.startswith("untrusted comment")][0]
    return base64.b64decode(line)[2:10]


def keyid_status(pubkey_b64, sig_b64):
    try:
        pub = keyid(pubkey_b64)
        sig = keyid(sig_b64.strip())
    except Exception as e:  # noqa: BLE001 - any parse failure is "could not parse"
        print(f"could not parse pubkey/.sig: {e}", file=sys.stderr)
        return 2
    return 0 if pub == sig else 1


def manifest_version(text):
    try:
        v = json.loads(text)["version"]
    except Exception:  # noqa: BLE001
        return None
    return v if isinstance(v, str) and parse_semver(v) else None


# ---- CLI --------------------------------------------------------------------------------

def cmd_notes(a):
    with open(a.changelog) as fh:
        notes = build_notes(fh.read(), a.upto, a.after)
    if a.out:
        with open(a.out, "w") as fh:
            fh.write(notes)
    else:
        sys.stdout.write(notes)
    return 0


def cmd_manifest(a):
    if os.path.exists(a.sig):
        with open(a.sig) as fh:
            sig = fh.read().strip()
    else:
        sig = "<signature>" if a.dry_run else ""
    if not sig:
        print("missing updater signature", file=sys.stderr)
        return 1
    with open(a.notes) as fh:
        notes = fh.read()
    write_manifest(a.out, build_manifest(a.version, notes, sig, a.url, a.pub_date))
    return 0


def cmd_newer(a):
    try:
        return 0 if is_newer(a.a, a.b) else 1
    except ValueError as e:
        print(e, file=sys.stderr)
        return 2


def cmd_keyid(a):
    try:
        with open(a.sig) as fh:
            sig = fh.read()
    except OSError as e:
        print(f"could not parse pubkey/.sig: {e}", file=sys.stderr)
        return 2
    return keyid_status(a.pubkey, sig)


def cmd_version(a):
    text = open(a.file).read() if a.file else sys.stdin.read()
    v = manifest_version(text)
    if v is None:
        return 2
    print(v)
    return 0


# ---- self-test --------------------------------------------------------------------------

CHANGELOG_FIXTURE = """# Changelog

## [Unreleased]

### Added

- Not released yet.

## [0.15.1] - 2026-11-20

### Fixed

- Fixed thing B.

### Internal

- Tooling B.

## [0.15.0] - 2026-11-10

### Added

- Feature A.

### Internal

- Tooling A.

### Changed

- Change A.

## [0.14.0] - 2026-11-05

### Added

- Old feature.
"""


def _minisign_blob(key_id, comment="untrusted comment: test"):
    payload = base64.b64encode(b"Ed" + key_id + b"\x00" * 32).decode()
    return base64.b64encode(f"{comment}\n{payload}\n".encode()).decode()


def self_test():
    failures = []

    def check(name, cond):
        print(("ok   " if cond else "FAIL ") + name)
        if not cond:
            failures.append(name)

    cl = CHANGELOG_FIXTURE
    one = build_notes(cl, "0.15.1", "0.15.0")
    check("notes: single version is its body only",
          one == "\n### Fixed\n\n- Fixed thing B.\n\n")
    check("notes: no --after means just X", build_notes(cl, "0.15.1") == one)
    check("notes: unknown --after means just X", build_notes(cl, "0.15.1", "<unknown>") == one)
    both = build_notes(cl, "0.15.1", "0.14.0")
    check("notes: skipped early combines newest first",
          both.index("Nixon 0.15.1") < both.index("Fixed thing B")
          < both.index("Nixon 0.15.0") < both.index("Feature A"))
    check("notes: combined excludes the version already advertised", "Old feature" not in both)
    check("notes: combined excludes [Unreleased]", "Not released yet" not in both)
    check("notes: ### Internal stripped in every section",
          "Tooling" not in both and "### Internal" not in both)
    check("notes: a subsection after ### Internal survives", "Change A" in both)
    check("notes: --upto bounds the range", "Fixed thing B" not in build_notes(cl, "0.15.0", "0.14.0"))
    check("notes: fallback when X has no section", build_notes(cl, "0.16.0", "0.15.1") == "Nixon v0.16.0\n")
    check("notes: fallback when X is all Internal",
          build_notes("## [1.0.0] - d\n\n### Internal\n\n- x\n", "1.0.0") == "Nixon v1.0.0\n")

    check("newer: greater", is_newer("0.15.0", "0.14.9"))
    check("newer: numeric, not lexical", is_newer("0.10.0", "0.9.0"))
    check("newer: equal is not newer", not is_newer("0.14.0", "0.14.0"))
    check("newer: older is not newer", not is_newer("0.13.1", "0.14.0"))
    check("newer: missing feed counts as older", is_newer("0.1.0", None) and is_newer("0.1.0", ""))
    check("newer: unknown feed counts as older", is_newer("0.1.0", "<unknown>"))
    try:
        is_newer("0.1", "0.0.1")
        check("newer: rejects a bad version", False)
    except ValueError:
        check("newer: rejects a bad version", True)

    k1, k2 = b"\x01" * 8, b"\x02" * 8
    check("keyid: match", keyid_status(_minisign_blob(k1), _minisign_blob(k1, "untrusted comment: sig")) == 0)
    check("keyid: mismatch", keyid_status(_minisign_blob(k1), _minisign_blob(k2)) == 1)
    check("keyid: unparseable", keyid_status(_minisign_blob(k1), "not base64!!") == 2)
    check("keyid: placeholder pubkey is unparseable", keyid_status("<pubkey-not-yet-configured>", _minisign_blob(k1)) == 2)

    m = build_manifest("1.2.3", "notes\n\n", "SIG", "https://x/y.tar.gz", "2026-01-01T00:00:00Z")
    with tempfile.TemporaryDirectory() as d:
        out = os.path.join(d, "m.json")
        write_manifest(out, m)
        raw = open(out).read()
    check("manifest: shape, key order, indent 2, no trailing newline", raw == (
        '{\n  "version": "1.2.3",\n  "notes": "notes",\n  "pub_date": "2026-01-01T00:00:00Z",\n'
        '  "platforms": {\n    "darwin-aarch64": {\n      "signature": "SIG",\n'
        '      "url": "https://x/y.tar.gz"\n    }\n  }\n}'))
    check("manifest: non-ASCII notes are escaped as before",
          "\\u2014" in json.dumps(build_manifest("1.0.0", "a — b", "S", "u", "d"), indent=2))
    check("version: reads a manifest", manifest_version(raw) == "1.2.3")
    check("version: rejects non-manifests",
          manifest_version("<html>") is None and manifest_version('{"version":"x"}') is None)

    print(f"\n{'FAILED' if failures else 'passed'}: {len(failures)} failure(s)")
    return 1 if failures else 0


def main(argv):
    if argv[:1] == ["--self-test"]:
        return self_test()
    p = argparse.ArgumentParser(prog="release_feed.py", description=__doc__.split("\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)

    n = sub.add_parser("notes")
    n.add_argument("--changelog", required=True)
    n.add_argument("--upto", required=True)
    n.add_argument("--after")
    n.add_argument("--out")
    n.set_defaults(fn=cmd_notes)

    m = sub.add_parser("manifest")
    m.add_argument("--version", required=True)
    m.add_argument("--notes", required=True)
    m.add_argument("--sig", required=True)
    m.add_argument("--url", required=True)
    m.add_argument("--out", required=True)
    m.add_argument("--dry-run", action="store_true")
    m.add_argument("--pub-date")
    m.set_defaults(fn=cmd_manifest)

    w = sub.add_parser("newer")
    w.add_argument("a")
    w.add_argument("b", nargs="?")
    w.set_defaults(fn=cmd_newer)

    k = sub.add_parser("keyid-match")
    k.add_argument("--pubkey", required=True)
    k.add_argument("--sig", required=True)
    k.set_defaults(fn=cmd_keyid)

    v = sub.add_parser("version")
    v.add_argument("--file")
    v.set_defaults(fn=cmd_version)

    a = p.parse_args(argv)
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
