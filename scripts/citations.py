#!/usr/bin/env python3
"""Rule-citation check with a test-hash lock. Driven by `just check-citations`
and `just citations-lock-update`; the justfile fetches the pinned artefacts and
passes them in as `LABEL=ZIP` arguments, where LABEL is `cen@…`, `peppol@…` or
`kosit@…`.

1. Every rule id is read from the artefacts with its rule context, its test and
   its flag. A CEN abstract rule (`test="$BR-CL-08"`, `context="$Invoice"`) is
   resolved through its UBL binding (`<param name="BR-CL-08" value="…"/>`), so a
   change to the binding is a change to the rule. Whitespace is normalised and
   nothing else is.
2. The published tree is scanned for every id whose prefix any pinned artefact
   uses, so a new family of ids is covered without editing a regex here.
3. A cited id that no artefact defines fails (unless it is cited *because* it
   does not exist, see ABSENT). So does an allow-listed absent id that now
   exists.
4. Each (artefact, cited id) is hashed over (context, test, flag) and compared
   with `citations.lock` in both directions: a changed hash, a missing entry
   and a stale entry each fail, naming the id and showing both expressions.
"""
import difflib
import hashlib
import os
import re
import sys
import xml.etree.ElementTree as ET
import zipfile

LOCK = "citations.lock"
SCAN = ["src", "README.md", "site/content", "tests", "examples", "benches"]
SCH = "{http://purl.oclc.org/dsdl/schematron}"
# Cited precisely because they do not exist: `B` is the one category without a
# rate rule, a zero-tax rule or a reason rule, and there is no BR-CO-25.
ABSENT = {"BR-B-05", "BR-B-09", "BR-B-10", "BR-CO-25"}
# (abstract rules, binding) pairs and concrete files, per artefact kind.
FILES = {
    "cen": [("ubl/schematron/abstract/EN16931-model.sch", "ubl/schematron/UBL/EN16931-UBL-model.sch"),
            ("ubl/schematron/abstract/EN16931-syntax.sch", "ubl/schematron/UBL/EN16931-UBL-syntax.sch"),
            ("ubl/schematron/codelist/EN16931-UBL-codes.sch", None)],
    "peppol": [("rules/sch/PEPPOL-EN16931-UBL.sch", None)],
    "kosit": [("schematron/ubl/XRechnung-UBL-validation.sch", None)],
}


def norm(s):
    return " ".join((s or "").split())


def member(zf, suffix):
    names = [n for n in zf.namelist() if n.endswith(suffix)]
    if len(names) != 1:
        sys.exit(f"{zf.filename}: expected one member ending in {suffix}, found {names}")
    return zf.read(names[0])


def rules(label, path):
    """id → sorted list of (context, test, flag), binding parameters resolved."""
    out = {}
    with zipfile.ZipFile(path) as zf:
        for rules_file, binding in FILES[label.split("@")[0]]:
            subst = lambda s: s  # noqa: E731
            if binding:
                params = {p.get("name"): p.get("value") for p in ET.fromstring(member(zf, binding)).iter(SCH + "param")}
                pat = re.compile(r"\$(" + "|".join(map(re.escape, sorted(params, key=len, reverse=True))) + r")(?![\w.-])")
                subst = lambda s, pat=pat, params=params: pat.sub(lambda m: params[m.group(1)], s or "")  # noqa: E731
            for rule in ET.fromstring(member(zf, rules_file)).iter(SCH + "rule"):
                ctx = norm(subst(rule.get("context")))
                for a in list(rule.iter(SCH + "assert")) + list(rule.iter(SCH + "report")):
                    if a.get("id"):
                        out.setdefault(a.get("id"), []).append((ctx, norm(subst(a.get("test"))), norm(a.get("flag"))))
    return {k: sorted(v) for k, v in out.items()}


def entry(occurrences):
    ctx, test, flag = (" ‖ ".join(o[i] for o in occurrences) for i in range(3))
    return hashlib.sha256(f"{ctx}\n{test}\n{flag}".encode()).hexdigest(), flag, ctx, test


def cited(prefixes):
    alt = "|".join(map(re.escape, sorted(prefixes, key=len, reverse=True)))
    rx = re.compile(r"(?<![A-Za-z0-9_-])(?:" + alt + r")[0-9]+(?:-[a-z])?(?![A-Za-z0-9_])")
    found = set()
    for root in SCAN:
        files = [root] if os.path.isfile(root) else [os.path.join(d, f) for d, _, fs in os.walk(root) for f in fs]
        for f in files:
            if f.endswith((".rs", ".md", ".html", ".toml")):
                with open(f, encoding="utf-8", errors="replace") as fh:
                    found.update(rx.findall(fh.read()))
    return found


def read_lock():
    lock = {}
    if os.path.exists(LOCK):
        for line in open(LOCK, encoding="utf-8"):
            if line.strip() and not line.startswith("#"):
                art, rid, sha, flag, ctx, test = line.rstrip("\n").split("\t")
                lock[(art, rid)] = (sha, flag, ctx, test)
    return lock


def show(tag, e):
    if len(e[3]) + len(e[2]) < 1500:
        print(f"    {tag}: flag={e[1]!r}\n      context: {e[2]}\n      test:    {e[3]}", file=sys.stderr)
    else:
        print(f"    {tag}: flag={e[1]!r}, context {len(e[2])} chars, test {len(e[3])} chars", file=sys.stderr)


def diff(old, new):
    show("locked", old)
    show("now   ", new)
    for i, name in ((2, "context"), (3, "test")):
        if old[i] != new[i] and len(old[2]) + len(old[3]) >= 1500:
            d = [t for t in difflib.ndiff(old[i].split(), new[i].split()) if t[:1] in "+-"]
            print(f"      {name} tokens: " + " ".join(d[:60]) + (" …" if len(d) > 60 else ""), file=sys.stderr)


def main():
    mode, args = sys.argv[1], sys.argv[2:]
    defs = {}
    for a in args:
        label, path = a.split("=", 1)
        defs[label] = rules(label, path)
    known = set().union(*defs.values())
    prefixes = {re.sub(r"[0-9]+(-[a-z])?$", "", k) for k in known}
    prefixes = {p for p in prefixes if p != "" and p[-1] in "-ABCDEFGHIJKLMNOPQRSTUVWXYZ" and "-" in p and p[0].isupper()}
    ids = cited(prefixes)
    fail = False
    missing = sorted(ids - known - ABSENT)
    if missing:
        fail = True
        print("✗ cited rule ids that no pinned artefact defines: " + " ".join(missing), file=sys.stderr)
    revived = sorted(ABSENT & known)
    if revived:
        fail = True
        print("✗ allow-listed as absent but now defined (cite it properly): " + " ".join(revived), file=sys.stderr)
    want = {(lab, rid): entry(d[rid]) for lab, d in defs.items() for rid in ids if rid in d}
    lock = read_lock()
    changes = []
    for key in sorted(set(want) | set(lock)):
        old, new = lock.get(key), want.get(key)
        if old and entry([(old[2], old[3], old[1])])[0] != old[0]:
            changes.append((key, "lock entry does not hash to its own sha256 (hand-edited?)", old, new))
        elif old is None:
            changes.append((key, "not in the lock", None, new))
        elif new is None:
            changes.append((key, "stale lock entry (no longer cited, or no longer defined there)", old, None))
        elif old[0] != new[0]:
            changes.append((key, "rule changed under the same id", old, new))
    for (art, rid), why, old, new in changes:
        print(f"{'✗' if mode == 'check' else '~'} {rid} in {art}: {why}", file=sys.stderr)
        if old and new:
            diff(old, new)
        elif new and mode == "update":
            show("new   ", new)
    if mode == "update":
        if missing or revived:
            sys.exit("refusing to write the lock while citations are broken")
        with open(LOCK, "w", encoding="utf-8") as fh:
            fh.write("# Citation test-hash lock: written by `just citations-lock-update`, read by\n"
                     "# `just check-citations`. One line per (artefact, cited rule id):\n"
                     "# artefact  id  sha256(context \\n test \\n flag)  flag  context  test\n"
                     "# Binding parameters are resolved and whitespace normalised. Do not edit by hand.\n")
            for (art, rid), e in sorted(want.items()):
                fh.write("\t".join((art, rid) + e) + "\n")
        print(f"✓ wrote {LOCK}: {len(want)} entries for {len(ids)} cited ids ({len(changes)} changed)")
        return
    if changes:
        fail = True
        print("  A rule's meaning moved under a cited id. Re-read it, decide whether any answer of the\n"
              "  crate moves, then run `just citations-lock-update`.", file=sys.stderr)
    if fail:
        sys.exit(1)
    print(f"✓ {len(ids)} cited rule ids examined (any prefix the artefacts use), {len(want)} "
          f"(artefact, id) entries locked and unchanged: {', '.join(defs)}")


if __name__ == "__main__":
    main()
