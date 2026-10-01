#!/usr/bin/env python3
"""Measure strata against itself and keep a history, chart and README table up to date.

    scripts/metrics.py run [--rev HEAD] [--corpus self|cirrocumulus] [--corpus-rev HEAD] [--label TEXT]
                            [--review standards=N,spec=N,correctness=N]
    scripts/metrics.py render

`run` checks out <rev> into a throwaway worktree (so results are tied to a commit, and
uncommitted changes are NOT measured), builds strata there, indexes the checkout, and compares
the result with rust-analyzer's SCIP index of the same checkout, which is the independent
ground truth. It appends (or replaces, for the same rev+label) one line in
docs/metrics/history.jsonl, then re-renders docs/metrics/chart.svg and the README table.

Recall definitions (all against SCIP, Rust only; fixtures are excluded from both sides):
  symbols    fraction of SCIP fn/method/type definitions that strata lists at the same path:line
  call_site  fraction of SCIP call references to workspace functions where strata recorded an
             edge near that line naming the same function
  target     as call_site, but strata's resolved/candidate targets must include the right function
  false_external  call sites to workspace functions that strata only reported as `external`
Stdlib only; needs cargo, git and rust-analyzer on PATH.
"""

import argparse
import datetime
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
METRICS = ROOT / "docs/metrics"
# `self` scores strata on its own commit; any other corpus is an external repo scored with the
# strata build under test. Add a corpus by adding an entry (its history/chart files follow).
CORPORA = {
    "self": {"title": "strata (self-analysis)", "path": None},
    "cirrocumulus": {
        "title": "cirrocumulus (external corpus)",
        "path": Path(os.environ.get("CIRRO_PATH", Path.home() / "cs/cirrocumulus")),
    },
}
README = ROOT / "README.md"
WORK = ROOT / "target/metrics"
LINE_WINDOW = 8  # a call expression may start this many lines above its method name


def sh(cmd, cwd=None, env=None, check=True):
    r = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True)
    if check and r.returncode != 0:
        sys.exit(f"command failed: {' '.join(map(str, cmd))}\n{r.stderr}")
    return r.stdout


# ---------------------------------------------------------------- SCIP (protobuf) decoding


def _varint(b, i):
    r = s = 0
    while True:
        c = b[i]
        i += 1
        r |= (c & 0x7F) << s
        if c < 0x80:
            return r, i
        s += 7


def _fields(b):
    i, n = 0, len(b)
    while i < n:
        k, i = _varint(b, i)
        f, w = k >> 3, k & 7
        if w == 0:
            v, i = _varint(b, i)
        elif w == 2:
            ln, i = _varint(b, i)
            v, i = b[i : i + ln], i + ln
        elif w == 1:
            v, i = b[i : i + 8], i + 8
        elif w == 5:
            v, i = b[i : i + 4], i + 4
        else:
            raise ValueError(f"unsupported wire type {w}")
        yield f, v


def _packed(b):
    i, out = 0, []
    while i < len(b):
        v, i = _varint(b, i)
        out.append(v)
    return out


def read_scip(path):
    """-> (definitions: {symbol: [(path, line1)]}, references: [(path, line1, endcol, symbol)])."""
    data = Path(path).read_bytes()
    defs, refs = {}, []
    for f, doc in _fields(data):
        if f != 2:
            continue
        rel, occs = None, []
        for df, dv in _fields(doc):
            if df == 1:
                rel = dv.decode()
            elif df == 2:
                occs.append(dv)
        for occ in occs:
            rng, sym, roles = [], "", 0
            for of, ov in _fields(occ):
                if of == 1:
                    rng = _packed(ov)
                elif of == 2:
                    sym = ov.decode()
                elif of == 3:
                    roles = ov
            if not sym or sym.startswith("local ") or len(rng) < 3:
                continue
            line1, endcol = rng[0] + 1, (rng[2] if len(rng) == 3 else rng[3])
            if roles & 1:
                defs.setdefault(sym, []).append((rel, line1))
            else:
                refs.append((rel, line1, endcol, sym))
    return defs, refs


def descriptors(symbol):
    """The descriptor part of a SCIP symbol (after scheme, manager, package, version)."""
    return symbol.split(" ", 4)[4] if symbol.count(" ") >= 4 else symbol


def is_fn(symbol):
    return descriptors(symbol).endswith("().")


def is_type(symbol):
    d = descriptors(symbol)
    # One `#` only: `a/Type#`. `Enum#Variant#` is a variant and `Trait#Assoc#` an associated type.
    return d.endswith("#") and d.count("#") == 1 and not d.startswith("impl#")


# ---------------------------------------------------------------- measuring one revision


def strata_json(binary, root, *args):
    return json.loads(sh([binary, *args], cwd=root))


def collect_edges(binary, root, symbols):
    seen, edges = set(), []
    names = sorted({s["name"] for s in symbols if s["kind"] in ("function", "method")})
    for name in names:
        for e in strata_json(binary, root, "callees", name, "--json"):
            key = (e["path"], e["line"], e["callee"])
            if key not in seen:
                seen.add(key)
                edges.append(e)
    return edges


def target_matches(target, desc, fn_name):
    """Does a strata target id (`a.b.Type.name`) name the SCIP definition `desc`?

    SCIP spells a definition as namespaces and types (`lang/rust/State#text().`). Every
    namespace and type in it must appear in the target; nested fns are filed under their
    module by SCIP, so the target may carry extra segments (`State.collect_locals.go`).
    """
    segs = target.split(".")
    if segs[-1] != fn_name:
        return False
    namespaces = [n for n in re.findall(r"`?([A-Za-z_][A-Za-z0-9_]*)`?/", desc) if n != "crate"]
    types = [t for t in re.findall(r"`?([A-Za-z_][A-Za-z0-9_]*)`?#", desc) if t != "impl"]
    return all(n in segs for n in namespaces + types)


def recall(root, symbols, edges, defs, refs):
    want_defs = {
        (p, ln)
        for sym, locs in defs.items()
        if is_fn(sym) or is_type(sym)
        for p, ln in locs
        if p.endswith(".rs")
    }
    have = {(s["path"], s["line"]) for s in symbols}
    sym_hit = want_defs & have

    workspace_fns = {s for s in defs if is_fn(s)}
    by_path = {}
    for e in edges:
        by_path.setdefault(e["path"], []).append(e)
    src_cache = {}

    def lines_of(rel):
        if rel not in src_cache:
            src_cache[rel] = (root / rel).read_text(errors="replace").splitlines()
        return src_cache[rel]

    calls = site_hits = target_hits = false_ext = 0
    missed = []
    for rel, line1, endcol, sym in refs:
        if sym not in workspace_fns or not rel.endswith(".rs"):
            continue
        lines = lines_of(rel)
        text = lines[line1 - 1] if line1 <= len(lines) else ""
        if re.match(r"\s*(pub(\([a-z]+\))?\s+)?use\s", text):
            continue
        after = text[endcol:]
        if not re.match(r"\s*(::<[^()]*>)?\s*\(", after):
            if after.strip() or line1 >= len(lines) or not lines[line1].lstrip().startswith("("):
                continue  # a function used as a value, not called
        calls += 1
        desc = descriptors(sym)
        name = re.findall(r"`([^`]+)`|([^/#.()\[\]!`]+)(?=\(\)\.$)", desc)
        fn_name = (name[-1][0] or name[-1][1]) if name else ""
        near = [
            e
            for e in by_path.get(rel, [])
            if line1 - LINE_WINDOW <= e["line"] <= line1
            and (
                e["callee"].split(".")[-1] == fn_name
                or any(t.split(".")[-1] == fn_name for t in e["targets"])
            )
        ]
        if near:
            site_hits += 1
        else:
            missed.append((rel, line1, fn_name, "no edge"))
        if any(
            e["state"] in ("resolved", "candidate")
            and any(target_matches(t, desc, fn_name) for t in e["targets"])
            for e in near
        ):
            target_hits += 1
        elif near:
            missed.append((rel, line1, fn_name, "wrong target: " + near[0]["state"]))
        if near and all(e["state"] == "external" for e in near):
            false_ext += 1
            missed.append((rel, line1, fn_name, "FALSE EXTERNAL: " + near[0]["callee"]))

    def ratio(a, b):
        return round(100 * a / b, 2) if b else None

    return {
        "recall": {
            "symbols": ratio(len(sym_hit), len(want_defs)),
            "call_site": ratio(site_hits, calls),
            "target": ratio(target_hits, calls),
        },
        "counts": {
            "scip_defs": len(want_defs),
            "defs_found": len(sym_hit),
            "scip_calls": calls,
            "call_site_hits": site_hits,
            "target_hits": target_hits,
            "false_external": false_ext,
        },
        "_missed": missed,
        "_missed_defs": sorted(want_defs - have),
    }


def prepare_corpus(name, spec, tool_wt, corpus_rev):
    """-> (root to index, corpus rev, corpus subject, env for rust-analyzer)."""
    if spec["path"] is None:
        return tool_wt, None, None, None
    repo = spec["path"]
    sha = sh(["git", "rev-parse", "--short", corpus_rev], cwd=repo).strip()
    subject = sh(["git", "log", "-1", "--format=%s", corpus_rev], cwd=repo).strip()
    root = WORK / f"corpus-{name}-{sha}"
    shutil.rmtree(root, ignore_errors=True)
    root.mkdir(parents=True)
    # `git archive` exports the committed tree without touching the corpus repo (its build
    # scripts may build into its own target/, so rust-analyzer must never run inside it).
    archive = subprocess.run(["git", "archive", sha], cwd=repo, capture_output=True, check=True)
    subprocess.run(["tar", "-x", "-C", str(root)], input=archive.stdout, check=True)
    # A pinned toolchain without rust-analyzer would make the rustup proxy refuse to run it.
    (root / "rust-toolchain.toml").unlink(missing_ok=True)
    env = {**os.environ, "CARGO_TARGET_DIR": str(WORK / f"ra-target-{name}")}
    return root, sha, subject, env


def measure(rev, corpus, corpus_rev, label, verbose):
    spec = CORPORA[corpus]
    sha = sh(["git", "rev-parse", "--short", rev], cwd=ROOT).strip()
    subject = sh(["git", "log", "-1", "--format=%s", rev], cwd=ROOT).strip()
    wt = WORK / f"wt-{sha}"
    WORK.mkdir(parents=True, exist_ok=True)
    if wt.exists():
        sh(["git", "worktree", "remove", "--force", str(wt)], cwd=ROOT, check=False)
    sh(["git", "worktree", "add", "--detach", str(wt), sha], cwd=ROOT)
    try:
        env = {**os.environ, "CARGO_TARGET_DIR": str(WORK / "build")}
        sh(["cargo", "build", "-q", "-p", "strata-cli"], cwd=wt, env=env)
        binary = str(WORK / "build/debug/strata")
        root, corpus_sha, corpus_subject, ra_env = prepare_corpus(corpus, spec, wt, corpus_rev)

        symbols = strata_json(binary, root, "symbols", "", "--json")
        edges = collect_edges(binary, root, symbols)
        states = {}
        for e in edges:
            states[e["state"]] = states.get(e["state"], 0) + 1
        unindexed = {
            ln.split()[1]
            for ln in sh([binary, "files", "--unindexed"], cwd=root).splitlines()
            if ln.strip()
        }
        rels = [
            str(p.relative_to(root))
            for ext in ("*.rs", "*.py")
            for p in root.rglob(ext)
        ]
        # Judge by the path inside the corpus: the scratch checkout itself lives under target/.
        tracked = [p for p in rels if "/fixtures/" not in p and not p.startswith("target/")]

        scip = WORK / f"{corpus}-{sha}.scip"
        sh(["rust-analyzer", "scip", ".", "--output", str(scip)], cwd=root, env=ra_env, check=False)
        defs, refs = read_scip(scip)
        result = recall(root, symbols, edges, defs, refs)
    finally:
        sh(["git", "worktree", "remove", "--force", str(wt)], cwd=ROOT, check=False)

    if verbose:
        for m in result["_missed"][:80]:
            print("  missed call:", *m)
        for m in result["_missed_defs"][:25]:
            print("  missed def:", *m)
    record = {
        "rev": sha,
        "subject": subject,
        "label": label or subject,
        "date": datetime.date.today().isoformat(),
        "files_indexed": len([p for p in tracked if p not in unindexed]),
        "symbols": len(symbols),
        "edges": dict(sorted(states.items())),
        "recall": result["recall"],
        "counts": result["counts"],
        "review": None,
    }
    if corpus_sha:
        record.update(corpus_rev=corpus_sha, corpus_subject=corpus_subject)
    return record


# ---------------------------------------------------------------- history, chart, README


def hist_path(corpus):
    return METRICS / ("history.jsonl" if corpus == "self" else f"history-{corpus}.jsonl")


def chart_path(corpus):
    return METRICS / ("chart.svg" if corpus == "self" else f"chart-{corpus}.svg")


def load(corpus):
    p = hist_path(corpus)
    if not p.exists():
        return []
    return [json.loads(ln) for ln in p.read_text().splitlines() if ln.strip()]


def save(corpus, runs):
    p = hist_path(corpus)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text("".join(json.dumps(r, sort_keys=True) + "\n" for r in runs))


def review_total(r):
    rv = r.get("review")
    return sum(rv.values()) if rv else None


def fmt(v, pct=False):
    return "–" if v is None else (f"{v:.1f}%" if pct else str(v))


def table(runs, external=False):
    cols = "| Run | strata | " + ("corpus | " if external else "") + "What changed | Symbol recall "
    cols += "| Call-site recall | Target recall | False external | Unresolved "
    cols += "| Calls scored |\n" if external else "| Review findings (std / spec / correctness) |\n"
    head = cols + "| ---: | --- |" + (" --- |" if external else "") + " --- | ---: | ---: | ---: | ---: | ---: |"
    head += (" ---: |\n" if external else " --- |\n")
    rows = []
    for i, r in enumerate(runs, 1):
        rv = r.get("review")
        review = (
            f"{sum(rv.values())} ({rv.get('standards', 0)} / {rv.get('spec', 0)} / "
            f"{rv.get('correctness', 0)})"
            if rv
            else "–"
        )
        rc = r["recall"]
        row = f"| {i} | `{r['rev']}` | "
        if external:
            row += f"`{r.get('corpus_rev', '')}` | "
        row += (
            f"{r['label']} | {fmt(rc['symbols'], True)} | {fmt(rc['call_site'], True)} "
            f"| {fmt(rc['target'], True)} | {r['counts']['false_external']} "
            f"| {r['edges'].get('unresolved', 0)} | "
        )
        row += f"{r['counts']['scip_calls']} |" if external else f"{review} |"
        rows.append(row)
    return head + "\n".join(rows) + "\n"


SERIES = [("symbols", "Symbols", "s1"), ("call_site", "Call sites", "s2"), ("target", "Targets", "s3")]


def chart(runs, title, review_panel=True):
    W, H = 760, 500 if review_panel else 290
    x0, x1 = 64, W - 120
    n = len(runs)
    xs = [(x0 + x1) / 2] if n == 1 else [x0 + (x1 - x0) * i / (n - 1) for i in range(n)]
    vals = [r["recall"][k] for r in runs for k, _, _ in SERIES if r["recall"][k] is not None]
    lo = max(0, int((min(vals, default=50) - 5) // 10 * 10))
    top1, bot1 = 70, 230
    y1 = lambda v: bot1 - (v - lo) / (100 - lo) * (bot1 - top1)
    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" '
        'role="img" aria-labelledby="t d">',
        "<title id=\"t\">{title}: recall per run</title>",
        f'<desc id="d">{n} runs. Recall of symbols, call sites and call targets against '
        "rust-analyzer, and code-review finding counts. Values are in the table below the chart.</desc>",
        "<style>",
        ".bg{fill:#fcfcfb} text{font:12px system-ui,sans-serif;fill:#52514e} .h{font-weight:600;fill:#0b0b0b;font-size:13px}",
        ".v{fill:#0b0b0b;font-weight:600} .grid{stroke:#e4e3df;stroke-width:1} .ln{fill:none;stroke-width:2;stroke-linejoin:round}",
        ".s1{stroke:#2a78d6} .s2{stroke:#eb6834} .s3{stroke:#1baf7a}",
        ".d1{fill:#2a78d6} .d2{fill:#eb6834} .d3{fill:#1baf7a} .dot{stroke:#fcfcfb;stroke-width:2} .bar{fill:#2a78d6}",
        "@media (prefers-color-scheme:dark){.bg{fill:#1a1a19} text{fill:#c3c2b7} .h,.v{fill:#fff} .grid{stroke:#34332f}",
        ".s1{stroke:#3987e5} .s2{stroke:#d95926} .s3{stroke:#199e70} .d1{fill:#3987e5} .d2{fill:#d95926} .d3{fill:#199e70}",
        ".dot{stroke:#1a1a19} .bar{fill:#3987e5}}",
        "</style>",
        f'<rect class="bg" width="{W}" height="{H}"/>',
        f'<text class="h" x="{x0}" y="26">{title}: recall vs rust-analyzer (%, higher is better)</text>',
    ]
    # legend
    lx = x0
    for _, name, cls in SERIES:
        out.append(f'<line class="ln {cls}" x1="{lx}" x2="{lx + 18}" y1="46" y2="46"/>')
        out.append(f'<text x="{lx + 24}" y="50">{name}</text>')
        lx += 24 + 8 * len(name) + 22
    ticks = [lo + (100 - lo) * k / 4 for k in range(5)]
    for t in ticks:
        out.append(f'<line class="grid" x1="{x0}" x2="{x1}" y1="{y1(t):.1f}" y2="{y1(t):.1f}"/>')
        out.append(f'<text x="{x0 - 8}" y="{y1(t) + 4:.1f}" text-anchor="end">{t:g}</text>')
    labels = []
    for key, name, cls in SERIES:
        pts = [(xs[i], y1(r["recall"][key]), r["recall"][key]) for i, r in enumerate(runs)
               if r["recall"][key] is not None]
        if not pts:
            continue
        out.append(
            f'<polyline class="ln {cls}" points="{" ".join(f"{x:.1f},{y:.1f}" for x, y, _ in pts)}"/>'
        )
        for i, r in enumerate(runs):
            v = r["recall"][key]
            if v is None:
                continue
            out.append(
                f'<circle class="dot d{cls[1]}" cx="{xs[i]:.1f}" cy="{y1(v):.1f}" r="4"><title>'
                f"run {i + 1} ({r['rev']}): {name} recall {v:.1f}%</title></circle>"
            )
        labels.append([pts[-1][1], f"{name} {pts[-1][2]:.1f}%"])
    labels.sort()
    for j in range(1, len(labels)):  # keep direct labels from overlapping
        labels[j][0] = max(labels[j][0], labels[j - 1][0] + 14)
    for y, text in labels:
        out.append(f'<text x="{x1 + 12}" y="{y + 4:.1f}">{text}</text>')
    if not review_panel:
        for i, r in enumerate(runs):
            out.append(f'<text x="{xs[i]:.1f}" y="{bot1 + 20}" text-anchor="middle">run {i + 1}</text>')
            out.append(f'<text x="{xs[i]:.1f}" y="{bot1 + 36}" text-anchor="middle">{r["rev"]}</text>')
    else:
        top2, bot2 = 300, 430
        out.append(f'<text class="h" x="{x0}" y="{top2 - 26}">Code-review findings per run (lower is better)</text>')
        mx = max([review_total(r) or 0 for r in runs] + [1])
        out.append(f'<line class="grid" x1="{x0}" x2="{x1}" y1="{bot2}" y2="{bot2}"/>')
        for i, r in enumerate(runs):
            tot = review_total(r)
            if tot is None:
                out.append(f'<text x="{xs[i]:.1f}" y="{bot2 - 6}" text-anchor="middle">n/a</text>')
            else:
                h = max(2, tot / mx * (bot2 - top2 - 18))
                rv = r["review"]
                out.append(
                    f'<rect class="bar" x="{xs[i] - 16:.1f}" y="{bot2 - h:.1f}" width="32" height="{h:.1f}" rx="4">'
                    f'<title>run {i + 1} ({r["rev"]}): {tot} findings — standards {rv.get("standards", 0)}, '
                    f'spec {rv.get("spec", 0)}, correctness {rv.get("correctness", 0)}</title></rect>'
                )
                out.append(f'<text class="v" x="{xs[i]:.1f}" y="{bot2 - h - 5:.1f}" text-anchor="middle">{tot}</text>')
        for i, r in enumerate(runs):
            out.append(f'<text x="{xs[i]:.1f}" y="{bot2 + 20}" text-anchor="middle">run {i + 1}</text>')
            out.append(f'<text x="{xs[i]:.1f}" y="{bot2 + 36}" text-anchor="middle">{r["rev"]}</text>')
    out.append("</svg>")
    return "\n".join(out) + "\n"


def render_corpus(corpus, runs):
    spec = CORPORA[corpus]
    external = spec["path"] is not None
    chart_path(corpus).parent.mkdir(parents=True, exist_ok=True)
    chart_path(corpus).write_text(chart(runs, spec["title"], review_panel=not external))
    start, end = f"<!-- metrics-{corpus}:start -->", f"<!-- metrics-{corpus}:end -->"
    if corpus == "self":  # the original markers
        start, end = "<!-- metrics:start -->", "<!-- metrics:end -->"
    intro = ""
    if external:
        intro = (
            f"### {spec['title']}\n\nThe same scoring on a repository outside strata. Run 1 is "
            "strata's first look at it, before any cirrocumulus-specific fix; later runs include "
            "fixes for the gaps run 1 exposed, so read them as tuned, not blind. `strata` is the "
            "commit under test, `corpus` the commit of the repo it indexed.\n\n"
        )
    block = (
        f"{start}\n\n{intro}![{spec['title']} chart](docs/metrics/{chart_path(corpus).name})\n\n"
        f"{table(runs, external)}\n{end}"
    )
    text = README.read_text()
    if start in text:
        text = re.sub(re.escape(start) + ".*?" + re.escape(end), lambda _: block, text, flags=re.S)
    else:
        text = text.rstrip() + "\n\n" + block + "\n"
    README.write_text(text)


def render_all():
    for corpus in CORPORA:
        runs = load(corpus)
        if runs:
            render_corpus(corpus, runs)


def parse_review(s):
    out = {}
    for part in s.split(","):
        k, v = part.split("=")
        out[k.strip()] = int(v)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run", help="measure a revision and update history, chart and README")
    r.add_argument("--rev", default="HEAD", help="strata commit to build and test (default HEAD)")
    r.add_argument("--corpus", default="self", choices=sorted(CORPORA), help="what to index")
    r.add_argument("--corpus-rev", default="HEAD", help="commit of an external corpus (default HEAD)")
    r.add_argument("--label", help="short note on what changed (default: commit subject)")
    r.add_argument("--review", type=parse_review, help="findings from a code review, e.g. standards=11,spec=11,correctness=9")
    r.add_argument("-v", "--verbose", action="store_true", help="list missed calls and definitions")
    sub.add_parser("render", help="redraw charts and README from history only")
    args = ap.parse_args()

    if args.cmd == "run":
        runs = load(args.corpus)
        rec = measure(args.rev, args.corpus, args.corpus_rev, args.label, args.verbose)
        rec["review"] = args.review
        key = lambda x: (x["rev"], x.get("corpus_rev"), x["label"])
        for i, old in enumerate(runs):
            if key(old) == key(rec):
                rec["review"] = rec["review"] or old.get("review")
                runs[i] = rec
                break
        else:
            runs.append(rec)
        save(args.corpus, runs)
        print(json.dumps(rec, indent=2))
    render_all()


if __name__ == "__main__":
    main()
