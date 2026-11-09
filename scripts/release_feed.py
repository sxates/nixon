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
  feed-read --feed latest|early --http CODE --body F [--allow-missing] [--dry-run]
      Classify one live feed read (the Worker's HTTP status and body). Prints two lines:
      a state (ok | missing | unknown | refuse) and the version or a message. Only
      latest.json with --allow-missing may be "missing"; anything else that is not a
      manifest is a fault: refuse, or unknown with --dry-run.
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


EMPTY_FEED_ENV = "NIXON_RELEASE_EMPTY_FEED"


def classify_feed_read(feed, http, body, allow_missing=False, dry_run=False):
    """(state, value) for one live read of updates/<feed>.json through the Worker.

    The Worker serves latest.json whenever the early.json object is absent, so a 404/503
    on early.json is a fault, never "not published". On latest.json it is either a fault
    or a brand-new empty bucket, which cannot be told apart from here: it only counts as
    missing when the caller says so (allow_missing). A fault refuses a real run and is
    "unknown" in a dry run, so a feed can never be moved backwards on a bad read.
    """
    name = f"updates/{feed}.json"
    if http == "200":
        v = manifest_version(body)
        if v is not None:
            return "ok", v
        msg = f"{name} answered HTTP 200 but not with an updater manifest."
    elif http in ("404", "503"):
        if feed == "latest" and allow_missing:
            return "missing", ""
        if feed == "early":
            msg = (f"{name} answered HTTP {http}. The site Worker serves latest.json whenever "
                   "early.json is absent, so this is a fault, not an unpublished feed; treating "
                   "it as missing could move early.json backwards. Check the Worker's "
                   "/updates/early.json route (specs/0086 W0) and re-run.")
        else:
            msg = (f"{name} answered HTTP {http}. That is a fault or a brand-new empty bucket; "
                   "treating it as missing would drop the cumulative release notes and the "
                   "forward-only check. Check the Worker and the bucket, and re-run. Only if "
                   f"this really is the first release ever, re-run with {EMPTY_FEED_ENV}=1.")
    else:
        got = "no response" if http in ("", "000") else f"HTTP {http}"
        msg = f"Could not read {name} ({got}). Check the connection and re-run."
    return ("unknown" if dry_run else "refuse"), msg


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


def cmd_feed_read(a):
    try:
        with open(a.body) as fh:
            body = fh.read()
    except OSError:
        body = ""
    state, value = classify_feed_read(a.feed, a.http, body, a.allow_missing, a.dry_run)
    print(state)
    print(value)
    return 0


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

    good = '{"version": "0.15.1"}'
    c = classify_feed_read
    check("feed-read: 200 manifest is ok", c("early", "200", good) == ("ok", "0.15.1"))
    check("feed-read: 200 non-manifest refuses", c("latest", "200", "<html>")[0] == "refuse")
    for code in ("404", "503"):
        check(f"feed-read: early {code} refuses (Worker falls back, so it's a fault)",
              c("early", code, "")[0] == "refuse" and "backwards" in c("early", code, "")[1])
        check(f"feed-read: early {code} refuses even with --allow-missing",
              c("early", code, "", allow_missing=True)[0] == "refuse")
        check(f"feed-read: latest {code} refuses and names the override",
              c("latest", code, "")[0] == "refuse" and EMPTY_FEED_ENV in c("latest", code, "")[1])
        check(f"feed-read: latest {code} is missing only with --allow-missing",
              c("latest", code, "", allow_missing=True) == ("missing", ""))
    check("feed-read: no response refuses", c("latest", "000", "")[0] == "refuse")
    check("feed-read: dry run turns a fault into unknown",
          c("early", "503", "", dry_run=True)[0] == "unknown"
          and c("latest", "000", "", dry_run=True)[0] == "unknown")
    check("feed-read: dry run never invents missing", c("latest", "404", "", dry_run=True)[0] == "unknown")

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

    r = sub.add_parser("feed-read")
    r.add_argument("--feed", required=True, choices=["latest", "early"])
    r.add_argument("--http", required=True)
    r.add_argument("--body", required=True)
    r.add_argument("--allow-missing", action="store_true")
    r.add_argument("--dry-run", action="store_true")
    r.set_defaults(fn=cmd_feed_read)

    v = sub.add_parser("version")
    v.add_argument("--file")
    v.set_defaults(fn=cmd_version)

    a = p.parse_args(argv)
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
